//! El enrutador y las rutas que no necesitan estado.

pub mod archivos;
pub mod jobs;

use crate::estado::Estado;
use axum::Json;
use axum::routing::{get, post, put};
use axum::{Router, response::IntoResponse};
use dp_core::catalogo;
use serde_json::json;
use std::sync::Arc;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;

/// Tope duro del cuerpo de una petición: el archivo más grande que permite el
/// plan más alto.
///
/// El límite por plan se comprueba después, ya con los bytes recibidos, y eso
/// tiene una consecuencia honesta: un cliente del plan gratuito puede hacernos
/// recibir un giga antes de que lo rechacemos. Se arregla de raíz en la fase
/// 4, cuando la subida vaya directa al almacenamiento con una URL prefirmada
/// de verdad y los bytes no pasen nunca por este proceso.
pub const LIMITE_CUERPO: usize = 1024 * 1024 * 1024;

pub fn enrutador(estado: Arc<Estado>) -> Router {
    Router::new()
        .route("/health", get(salud))
        .route("/v1/operations", get(operaciones))
        // En Axum 0.8 los parámetros de ruta van entre llaves, no con dos
        // puntos como en las versiones anteriores.
        .route("/v1/files/presign", post(archivos::presign))
        .route("/v1/files/{id}", get(archivos::detalle))
        .route("/v1/files/{id}/download", get(archivos::descarga))
        .route(
            "/v1/files/{id}/content",
            // Misma ruta, dos verbos: subir con PUT y descargar con GET. Cada
            // uno exige una firma distinta, así que un enlace de descarga no
            // sirve para sobrescribir.
            put(archivos::subir_contenido).get(archivos::bajar_contenido),
        )
        .route("/v1/jobs", post(jobs::crear).get(jobs::listar))
        .route("/v1/jobs/{id}", get(jobs::detalle))
        .route("/v1/jobs/{id}/cancel", post(jobs::cancelar))
        .layer(RequestBodyLimitLayer::new(LIMITE_CUERPO))
        // Una traza por petición con su método, ruta y latencia. Es la base de
        // las métricas p50/p95 que pide la sección 12 del PDF.
        .layer(TraceLayer::new_for_http())
        .with_state(estado)
}

/// Sonda de vida para el orquestador. Sin autenticación a propósito: quien la
/// consulta es el balanceador, no un cliente.
async fn salud() -> impl IntoResponse {
    Json(json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") }))
}

/// El catálogo de operaciones sale del trait `Operacion` de `dp-core`, no de
/// una lista escrita a mano aquí. Registrar una operación nueva en el motor la
/// publica en la API automáticamente, y por eso no pueden desincronizarse.
async fn operaciones() -> impl IntoResponse {
    use crate::dto::{RespuestaLista, RespuestaOperacion};

    let data: Vec<RespuestaOperacion> = catalogo()
        .iter()
        .map(|op| RespuestaOperacion {
            name: op.nombre(),
            description: op.descripcion(),
            produces_file: op.produce_archivo(),
            minimum_credits: dp_dominio::creditos_estimados(op.nombre(), 0, false),
        })
        .collect();

    Json(RespuestaLista {
        count: data.len(),
        data,
    })
}

/// ¿Existe esta operación en el motor?
pub fn operacion_existe(nombre: &str) -> bool {
    catalogo().iter().any(|op| op.nombre() == nombre)
}
