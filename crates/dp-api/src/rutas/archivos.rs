//! Subida y descarga de archivos.

use crate::auth::Identidad;
use crate::dto::{
    ParametrosFirma, PeticionPresign, RespuestaArchivo, RespuestaDescarga, RespuestaPresign,
};
use crate::error::ErrorApi;
use crate::estado::Estado;
use crate::firma::{Accion, Firmante};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::IntoResponse;
use dp_dominio::{Archivo, IdArchivo, IdOrganizacion, TipoArchivo};
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// Cuántos bytes se leen para detectar el tipo. La firma de un PDF cabe en
/// cinco, pero un CSV necesita ver algo de la primera línea.
const BYTES_PARA_DETECTAR: usize = 4096;

/// `POST /v1/files/presign` — reserva un identificador y devuelve la URL de
/// subida.
///
/// El archivo se registra ANTES de tener contenido. Así el identificador ya
/// existe cuando se devuelve, la subida es idempotente (repetirla sobrescribe
/// el mismo objeto) y una subida que nunca llega deja un rastro que la tarea
/// de limpieza puede recoger.
///
/// El tope de bytes del plan se decide **aquí**, donde sabemos quién es el
/// cliente, y se firma dentro de la URL. El endpoint de subida solo obedece.
pub async fn presign(
    State(estado): State<Arc<Estado>>,
    identidad: Identidad,
    Json(peticion): Json<PeticionPresign>,
) -> Result<impl IntoResponse, ErrorApi> {
    let ahora = estado.reloj.ahora();
    let archivo = Archivo::pendiente(identidad.organizacion, &peticion.filename, ahora);

    let max_bytes = identidad.plan.limites().bytes_por_archivo;
    let recurso = Firmante::recurso_subida(&identidad.organizacion, &archivo.id, max_bytes);
    let (firma, expira) = estado.firmante.firmar(Accion::Subir, &recurso, ahora);

    let respuesta = RespuestaPresign {
        file_id: archivo.id,
        upload_url: format!(
            "{}/v1/files/{}/content?org={}&max={}&exp={}&sig={}",
            estado.base_publica, archivo.id, identidad.organizacion, max_bytes, expira, firma
        ),
        expires_at: Firmante::vencimiento(ahora),
    };

    estado.archivos.crear(archivo).await?;
    Ok((StatusCode::CREATED, Json(respuesta)))
}

/// `PUT /v1/files/{id}/content` — recibe los bytes.
///
/// No lleva cabecera de autorización: la firma de la URL es la credencial, y
/// cubre la acción, la organización, el archivo y el tamaño máximo. Es el
/// mismo modelo de una URL prefirmada de S3.
pub async fn subir_contenido(
    State(estado): State<Arc<Estado>>,
    Path(id): Path<IdArchivo>,
    Query(firma): Query<ParametrosFirma>,
    cuerpo: Bytes,
) -> Result<impl IntoResponse, ErrorApi> {
    let ahora = estado.reloj.ahora();

    // Sin `max` no hay nada que verificar: es una URL de descarga usada con
    // PUT, o una URL manipulada.
    let max_bytes = firma.max.ok_or(ErrorApi::FirmaInvalida)?;
    let recurso = Firmante::recurso_subida(&firma.org, &id, max_bytes);
    estado
        .firmante
        .verificar(Accion::Subir, &recurso, firma.exp, &firma.sig, ahora)?;

    let bytes = cuerpo.len() as u64;
    if bytes > max_bytes {
        return Err(ErrorApi::ArchivoDemasiadoGrande {
            bytes,
            limite: max_bytes,
        });
    }

    let mut archivo = buscar(&estado, firma.org, id).await?;

    // Sección 9 del PDF: el tipo se deduce del contenido, no del nombre ni de
    // la cabecera Content-Type que manda el cliente.
    let asomo = &cuerpo[..cuerpo.len().min(BYTES_PARA_DETECTAR)];
    let tipo = TipoArchivo::detectar(asomo);

    let mut hasher = Sha256::new();
    hasher.update(&cuerpo);
    let huella = hex::encode(hasher.finalize());

    estado.almacen.guardar(&archivo.clave, &cuerpo).await?;
    archivo.confirmar(bytes, tipo, huella);

    let respuesta = RespuestaArchivo::from(&archivo);
    estado.archivos.actualizar(archivo).await?;

    tracing::info!(archivo = %id, bytes, tipo = tipo.etiqueta(), "contenido recibido");

    Ok(Json(respuesta))
}

/// `GET /v1/files/{id}` — metadata del archivo.
pub async fn detalle(
    State(estado): State<Arc<Estado>>,
    identidad: Identidad,
    Path(id): Path<IdArchivo>,
) -> Result<impl IntoResponse, ErrorApi> {
    let archivo = buscar(&estado, identidad.organizacion, id).await?;
    Ok(Json(RespuestaArchivo::from(&archivo)))
}

/// `GET /v1/files/{id}/download` — entrega una URL temporal de descarga.
pub async fn descarga(
    State(estado): State<Arc<Estado>>,
    identidad: Identidad,
    Path(id): Path<IdArchivo>,
) -> Result<impl IntoResponse, ErrorApi> {
    let archivo = buscar(&estado, identidad.organizacion, id).await?;

    // No se entrega una URL para algo que todavía no tiene bytes: el cliente
    // recibiría un error al usarla y no sabría por qué.
    if !archivo.esta_disponible() {
        return Err(ErrorApi::ArchivoSinContenido(id));
    }

    let ahora = estado.reloj.ahora();
    let recurso = Firmante::recurso_descarga(&identidad.organizacion, &id);
    let (firma, expira) = estado.firmante.firmar(Accion::Descargar, &recurso, ahora);

    Ok(Json(RespuestaDescarga {
        file_id: id,
        download_url: format!(
            "{}/v1/files/{}/content?org={}&exp={}&sig={}",
            estado.base_publica, id, identidad.organizacion, expira, firma
        ),
        expires_at: Firmante::vencimiento(ahora),
    }))
}

/// `GET /v1/files/{id}/content` — los bytes.
pub async fn bajar_contenido(
    State(estado): State<Arc<Estado>>,
    Path(id): Path<IdArchivo>,
    Query(firma): Query<ParametrosFirma>,
) -> Result<impl IntoResponse, ErrorApi> {
    let ahora = estado.reloj.ahora();
    let recurso = Firmante::recurso_descarga(&firma.org, &id);
    estado
        .firmante
        .verificar(Accion::Descargar, &recurso, firma.exp, &firma.sig, ahora)?;

    let archivo = buscar(&estado, firma.org, id).await?;
    if !archivo.esta_disponible() {
        return Err(ErrorApi::ArchivoSinContenido(id));
    }

    // Cargar el archivo entero en memoria para servirlo es aceptable con
    // archivos de megabytes y no lo es con archivos de gigabytes. En la fase 4
    // este endpoint desaparece: la URL prefirmada apunta directo al
    // almacenamiento y los bytes no vuelven a pasar por este proceso.
    let contenido = estado.almacen.leer(&archivo.clave).await?;

    let mut cabeceras = HeaderMap::new();
    cabeceras.insert(
        header::CONTENT_TYPE,
        mime_de(archivo.tipo)
            .parse()
            .expect("los tipos MIME son constantes validas"),
    );
    // `attachment` obliga al navegador a descargar en vez de interpretar el
    // archivo. Sección 9 del PDF: nada de ejecutar lo que sube el usuario.
    cabeceras.insert(
        header::CONTENT_DISPOSITION,
        format!("attachment; filename=\"{}\"", nombre_ascii(&archivo))
            .parse()
            .expect("el nombre ya viene saneado"),
    );

    Ok((cabeceras, contenido))
}

fn mime_de(tipo: TipoArchivo) -> &'static str {
    match tipo {
        TipoArchivo::Csv => "text/csv; charset=utf-8",
        TipoArchivo::Json => "application/json",
        TipoArchivo::Pdf => "application/pdf",
        // Lo desconocido se declara binario: así ningún navegador intenta
        // interpretarlo como HTML y ejecutar lo que lleve dentro.
        TipoArchivo::Desconocido => "application/octet-stream",
    }
}

/// El nombre ya viene sin rutas ni caracteres de control, pero una cabecera
/// HTTP solo admite ASCII, así que hay que quitar los acentos también.
fn nombre_ascii(archivo: &Archivo) -> String {
    let limpio: String = archivo
        .nombre_original
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' '))
        .collect();

    if limpio.trim().is_empty() {
        format!("{}.{}", archivo.id, archivo.tipo.etiqueta())
    } else {
        limpio
    }
}

/// Busca el archivo exigiendo la organización.
///
/// Un archivo de otro cliente devuelve 404 y no 403, deliberadamente: un 403
/// confirmaría que el identificador existe.
async fn buscar(
    estado: &Estado,
    organizacion: IdOrganizacion,
    id: IdArchivo,
) -> Result<Archivo, ErrorApi> {
    estado
        .archivos
        .obtener(organizacion, id)
        .await?
        .ok_or(ErrorApi::ArchivoNoEncontrado(id))
}
