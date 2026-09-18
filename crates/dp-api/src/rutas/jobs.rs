//! Creación y consulta de jobs.

use crate::auth::Identidad;
use crate::dto::{FiltroJobs, PeticionCrearJob, RespuestaJob, RespuestaLista};
use crate::error::ErrorApi;
use crate::estado::Estado;
use crate::rutas::operacion_existe;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use dp_dominio::{IdJob, Job, TipoArchivo, creditos_estimados};
use std::sync::Arc;

/// Cuántos jobs devuelve el listado si el cliente no pide otra cosa.
const LIMITE_LISTADO: usize = 20;
const LIMITE_LISTADO_MAXIMO: usize = 100;

/// `POST /v1/jobs`
///
/// El orden de las validaciones no es casual: va de lo más barato a lo más
/// caro. Comprobar el nombre de la operación no toca la base de datos;
/// contar los jobs en vuelo, sí. Rechazar temprano es la diferencia entre
/// aguantar una avalancha de peticiones inválidas y caerse con ella.
pub async fn crear(
    State(estado): State<Arc<Estado>>,
    identidad: Identidad,
    Json(peticion): Json<PeticionCrearJob>,
) -> Result<impl IntoResponse, ErrorApi> {
    // 1. ¿Existe la operación? El catálogo sale del motor, no de una lista
    //    duplicada aquí.
    if !operacion_existe(&peticion.operation) {
        return Err(ErrorApi::OperacionDesconocida(peticion.operation));
    }

    let limites = identidad.plan.limites();

    // 2. ¿Existe el archivo, y es de este cliente? La organización va en la
    //    consulta, no como comprobación posterior.
    let archivo = estado
        .archivos
        .obtener(identidad.organizacion, peticion.file_id)
        .await?
        .ok_or(ErrorApi::ArchivoNoEncontrado(peticion.file_id))?;

    // 3. ¿Tiene contenido? Encolar un job sobre un archivo vacío es gastar un
    //    worker para descubrir lo que ya sabemos aquí.
    if !archivo.esta_disponible() {
        return Err(ErrorApi::ArchivoSinContenido(archivo.id));
    }

    // 4. ¿El tipo real del archivo sirve para esta operación?
    let esperado = tipo_que_exige(&peticion.operation);
    if archivo.tipo != esperado {
        return Err(ErrorApi::tipo_no_soportado(esperado, archivo.tipo));
    }

    // 5. ¿Le cabe en el plan? Este límite se comprobó al subir, pero un plan
    //    puede haber bajado de categoría desde entonces.
    if archivo.bytes > limites.bytes_por_archivo {
        return Err(ErrorApi::ArchivoDemasiadoGrande {
            bytes: archivo.bytes,
            limite: limites.bytes_por_archivo,
        });
    }

    // 6. La consulta que sí cuesta: cuántos jobs tiene sin terminar. Es lo que
    //    evita que un cliente monopolice todos los workers.
    let en_vuelo = estado.jobs.en_vuelo(identidad.organizacion).await?;
    if en_vuelo >= limites.jobs_en_vuelo {
        return Err(ErrorApi::DemasiadosJobs {
            en_vuelo,
            limite: limites.jobs_en_vuelo,
        });
    }

    // El cobro definitivo lo hace el worker con las filas realmente leídas:
    // la reserva es un techo. Sin saldo no se encola.
    let reservados = creditos_estimados(
        &peticion.operation,
        archivo.bytes,
        peticion.options.deduplica(),
    );

    let ahora = estado.reloj.ahora();
    let job = Job::nuevo(
        identidad.organizacion,
        peticion.operation,
        archivo.id,
        peticion.options,
        limites,
        reservados,
        ahora,
    );

    estado
        .libro
        .reservar(identidad.organizacion, job.id, reservados, ahora)
        .await?;

    let respuesta = RespuestaJob::from(&job);
    if let Err(error) = estado.jobs.crear(job).await {
        let _ = estado
            .libro
            .liberar(identidad.organizacion, respuesta.job_id, ahora)
            .await;
        return Err(error.into());
    }

    tracing::info!(
        job = %respuesta.job_id,
        operacion = %respuesta.operation,
        bytes = archivo.bytes,
        creditos = reservados,
        "job encolado"
    );

    Ok((StatusCode::ACCEPTED, Json(respuesta)))
}

/// `GET /v1/jobs/{id}`
pub async fn detalle(
    State(estado): State<Arc<Estado>>,
    identidad: Identidad,
    Path(id): Path<IdJob>,
) -> Result<impl IntoResponse, ErrorApi> {
    let job = estado
        .jobs
        .obtener(identidad.organizacion, id)
        .await?
        .ok_or(ErrorApi::JobNoEncontrado(id))?;

    Ok(Json(RespuestaJob::from(&job)))
}

/// `GET /v1/jobs`
pub async fn listar(
    State(estado): State<Arc<Estado>>,
    identidad: Identidad,
    Query(filtro): Query<FiltroJobs>,
) -> Result<impl IntoResponse, ErrorApi> {
    // El límite se acota siempre: sin este `min`, un cliente pediría un
    // millón de jobs y nos haría materializarlos todos en memoria.
    let limite = filtro
        .limit
        .unwrap_or(LIMITE_LISTADO)
        .clamp(1, LIMITE_LISTADO_MAXIMO);

    let jobs = estado
        .jobs
        .listar(identidad.organizacion, filtro.status, limite)
        .await?;

    let data: Vec<RespuestaJob> = jobs.iter().map(RespuestaJob::from).collect();
    Ok(Json(RespuestaLista {
        count: data.len(),
        data,
    }))
}

/// `POST /v1/jobs/{id}/cancel`
///
/// Solo funciona mientras el job siga encolado. Si ya lo tomó un worker,
/// devuelve 409: interrumpir un job en vuelo requiere que el worker colabore,
/// y eso es trabajo de una fase posterior.
pub async fn cancelar(
    State(estado): State<Arc<Estado>>,
    identidad: Identidad,
    Path(id): Path<IdJob>,
) -> Result<impl IntoResponse, ErrorApi> {
    let job = estado
        .jobs
        .cancelar(identidad.organizacion, id, estado.reloj.ahora())
        .await
        .map_err(|error| match error {
            // El repositorio no distingue "no existe" de "no es tuyo", y está
            // bien: aquí se traduce al 404 que ve el cliente.
            dp_dominio::ErrorRepositorio::NoEncontrado { .. } => ErrorApi::JobNoEncontrado(id),
            otro => ErrorApi::from(otro),
        })?;

    let _ = estado
        .libro
        .liberar(identidad.organizacion, id, estado.reloj.ahora())
        .await;
    tracing::info!(job = %id, "job cancelado por el cliente");
    Ok(Json(RespuestaJob::from(&job)))
}

/// Qué tipo de archivo exige cada operación.
///
/// Hoy todas son de CSV. Cuando entren las de PDF en la fase 5, esta función
/// es el único sitio que cambia, y el `match` sobre el prefijo del nombre
/// mantiene la convención `csv.*` / `pdf.*` que ya usa el catálogo.
fn tipo_que_exige(operacion: &str) -> TipoArchivo {
    match operacion.split('.').next() {
        Some("pdf") => TipoArchivo::Pdf,
        _ => TipoArchivo::Csv,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn las_operaciones_de_csv_exigen_un_csv() {
        assert_eq!(tipo_que_exige("csv.clean"), TipoArchivo::Csv);
        assert_eq!(tipo_que_exige("csv.inspect"), TipoArchivo::Csv);
    }

    #[test]
    fn las_operaciones_de_pdf_exigiran_un_pdf() {
        assert_eq!(tipo_que_exige("pdf.extract_text"), TipoArchivo::Pdf);
    }

    #[test]
    fn el_catalogo_del_motor_es_la_fuente_de_verdad() {
        assert!(operacion_existe("csv.clean"));
        assert!(operacion_existe("csv.inspect"));
        assert!(!operacion_existe("csv.inventada"));
    }
}
