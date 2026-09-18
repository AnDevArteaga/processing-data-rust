//! El error de la API y su traducción a HTTP.
//!
//! Todo el sistema decide el código de estado en un solo lugar: aquí. Ningún
//! manejador de ruta construye una respuesta de error a mano, así que es
//! imposible que dos endpoints reporten el mismo problema de forma distinta.
//!
//! El formato del cuerpo es exactamente la envoltura que ya produce el motor
//! en `dp-core::salida`. Un cliente que sepa leer el error de un job sabe leer
//! el error de la API.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use dp_core::{DetalleError, ErrorDp};
use dp_dominio::{
    ErrorAlmacen, ErrorRepositorio, IdArchivo, IdJob, TipoArchivo, TransicionInvalida,
};
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ErrorApi {
    #[error("falta o es invalida la credencial de acceso")]
    NoAutorizado,

    #[error("la operacion '{0}' no existe")]
    OperacionDesconocida(String),

    #[error("no existe el archivo '{0}'")]
    ArchivoNoEncontrado(IdArchivo),

    #[error("no existe el job '{0}'")]
    JobNoEncontrado(IdJob),

    #[error("el archivo '{0}' todavia no tiene contenido confirmado")]
    ArchivoSinContenido(IdArchivo),

    #[error("la operacion necesita un archivo {esperado} y este es {recibido}")]
    TipoNoSoportado {
        esperado: &'static str,
        recibido: &'static str,
    },

    #[error("el archivo pesa {bytes} bytes y tu plan permite {limite}")]
    ArchivoDemasiadoGrande { bytes: u64, limite: u64 },

    #[error("ya tienes {en_vuelo} jobs sin terminar y tu plan permite {limite}")]
    DemasiadosJobs { en_vuelo: usize, limite: usize },

    #[error("saldo insuficiente: hay {disponible} creditos y el job pide {pedido}")]
    SaldoInsuficiente { disponible: u64, pedido: u64 },

    #[error("no existe la api key '{0}'")]
    ClaveNoEncontrada(dp_dominio::IdApiKey),

    #[error("la firma de la URL no es valida")]
    FirmaInvalida,

    #[error("la URL temporal vencio")]
    FirmaVencida,

    #[error("peticion invalida: {0}")]
    Peticion(String),

    #[error(transparent)]
    Transicion(#[from] TransicionInvalida),

    #[error("fallo del almacenamiento de objetos: {0}")]
    Almacen(#[from] ErrorAlmacen),

    #[error("fallo del almacenamiento de datos: {0}")]
    Repositorio(ErrorRepositorio),

    /// El motor falló dentro de una operación síncrona.
    #[error(transparent)]
    Motor(#[from] ErrorDp),
}

/// `ErrorRepositorio` envuelve `TransicionInvalida`, y una transición inválida
/// es culpa del cliente (canceló algo que ya estaba en proceso), no un fallo
/// de la base de datos. Esta conversión desempaqueta ese caso para que no
/// termine reportado como un 500.
impl From<ErrorRepositorio> for ErrorApi {
    fn from(error: ErrorRepositorio) -> Self {
        match error {
            ErrorRepositorio::Transicion(t) => ErrorApi::Transicion(t),
            ErrorRepositorio::SaldoInsuficiente { disponible, pedido } => {
                ErrorApi::SaldoInsuficiente { disponible, pedido }
            }
            otro => ErrorApi::Repositorio(otro),
        }
    }
}

impl ErrorApi {
    /// Código estable. Es lo que se agrupa en las alertas; el mensaje puede
    /// cambiar entre versiones sin romper a nadie.
    ///
    /// El `match` sin rama por defecto es intencional: una variante nueva
    /// rompe la compilación aquí hasta que le asignes su código.
    pub fn codigo(&self) -> &'static str {
        match self {
            ErrorApi::NoAutorizado => "E_NO_AUTORIZADO",
            ErrorApi::OperacionDesconocida(_) => "E_OPERACION_DESCONOCIDA",
            ErrorApi::ArchivoNoEncontrado(_) => "E_ARCHIVO_NO_ENCONTRADO",
            ErrorApi::JobNoEncontrado(_) => "E_JOB_NO_ENCONTRADO",
            ErrorApi::ArchivoSinContenido(_) => "E_ARCHIVO_SIN_CONTENIDO",
            ErrorApi::TipoNoSoportado { .. } => "E_TIPO_NO_SOPORTADO",
            ErrorApi::ArchivoDemasiadoGrande { .. } => "E_ARCHIVO_DEMASIADO_GRANDE",
            ErrorApi::DemasiadosJobs { .. } => "E_DEMASIADOS_JOBS",
            ErrorApi::SaldoInsuficiente { .. } => "E_SALDO_INSUFICIENTE",
            ErrorApi::ClaveNoEncontrada(_) => "E_API_KEY_NO_ENCONTRADA",
            ErrorApi::FirmaInvalida => "E_FIRMA_INVALIDA",
            ErrorApi::FirmaVencida => "E_FIRMA_VENCIDA",
            ErrorApi::Peticion(_) => "E_PETICION_INVALIDA",
            ErrorApi::Transicion(_) => "E_TRANSICION_INVALIDA",
            ErrorApi::Almacen(_) => "E_ALMACEN",
            ErrorApi::Repositorio(_) => "E_REPOSITORIO",
            ErrorApi::Motor(error) => error.codigo(),
        }
    }

    pub fn estado_http(&self) -> StatusCode {
        match self {
            ErrorApi::NoAutorizado => StatusCode::UNAUTHORIZED,
            ErrorApi::FirmaInvalida | ErrorApi::FirmaVencida => StatusCode::FORBIDDEN,

            ErrorApi::ArchivoNoEncontrado(_) | ErrorApi::JobNoEncontrado(_) => {
                StatusCode::NOT_FOUND
            }

            // 409 y no 400: la petición estaba bien formada, pero el recurso no
            // está en un estado en el que se pueda hacer lo que pides.
            ErrorApi::ArchivoSinContenido(_) | ErrorApi::Transicion(_) => StatusCode::CONFLICT,

            ErrorApi::TipoNoSoportado { .. } => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            ErrorApi::ArchivoDemasiadoGrande { .. } => StatusCode::PAYLOAD_TOO_LARGE,
            // 429 con la semántica de "estás usando más de lo que tu plan
            // permite": el cliente debe esperar a que terminen sus jobs.
            ErrorApi::DemasiadosJobs { .. } => StatusCode::TOO_MANY_REQUESTS,
            ErrorApi::SaldoInsuficiente { .. } => StatusCode::PAYMENT_REQUIRED,
            ErrorApi::ClaveNoEncontrada(_) => StatusCode::NOT_FOUND,

            ErrorApi::OperacionDesconocida(_) | ErrorApi::Peticion(_) => StatusCode::BAD_REQUEST,

            ErrorApi::Almacen(_) | ErrorApi::Repositorio(_) => StatusCode::INTERNAL_SERVER_ERROR,

            // El motor ya sabe de quién es la culpa: reusamos su decisión en
            // vez de volver a clasificar sus errores aquí.
            ErrorApi::Motor(error) => {
                if error.es_culpa_del_cliente() {
                    StatusCode::BAD_REQUEST
                } else {
                    StatusCode::INTERNAL_SERVER_ERROR
                }
            }
        }
    }

    pub fn culpa_del_cliente(&self) -> bool {
        !self.estado_http().is_server_error()
    }

    /// Un error del cliente no se debe reintentar: el resultado sería igual.
    pub fn reintentable(&self) -> bool {
        !self.culpa_del_cliente()
    }

    fn causas(&self) -> Vec<String> {
        let mut causas = Vec::new();
        let mut actual = std::error::Error::source(self);
        while let Some(error) = actual {
            causas.push(error.to_string());
            actual = error.source();
        }
        causas
    }

    /// Ayuda para construir el error de tipo con etiquetas legibles.
    pub fn tipo_no_soportado(esperado: TipoArchivo, recibido: TipoArchivo) -> Self {
        ErrorApi::TipoNoSoportado {
            esperado: esperado.etiqueta(),
            recibido: recibido.etiqueta(),
        }
    }
}

/// La misma forma que produce el motor: `{ "ok": false, "error": { ... } }`.
#[derive(Serialize)]
struct SobreError {
    ok: bool,
    error: DetalleError,
}

impl IntoResponse for ErrorApi {
    fn into_response(self) -> Response {
        let estado = self.estado_http();

        // Los fallos internos se registran con nivel error porque hay que
        // mirarlos; los del cliente, con nivel info, porque son parte de la
        // operación normal de una API pública.
        if estado.is_server_error() {
            tracing::error!(codigo = self.codigo(), error = %self, "fallo interno");
        } else {
            tracing::info!(codigo = self.codigo(), error = %self, "peticion rechazada");
        }

        let cuerpo = SobreError {
            ok: false,
            error: DetalleError {
                codigo: self.codigo(),
                reintentable: self.reintentable(),
                culpa_del_cliente: self.culpa_del_cliente(),
                mensaje: self.to_string(),
                causas: self.causas(),
            },
        };

        (estado, Json(cuerpo)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn los_errores_del_cliente_no_son_reintentables() {
        let error = ErrorApi::OperacionDesconocida("pdf.ocr".to_string());
        assert_eq!(error.estado_http(), StatusCode::BAD_REQUEST);
        assert!(error.culpa_del_cliente());
        assert!(!error.reintentable());
    }

    #[test]
    fn los_fallos_internos_si_son_reintentables() {
        let error = ErrorApi::Repositorio(ErrorRepositorio::Interno("conexion caida".to_string()));
        assert_eq!(error.estado_http(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(!error.culpa_del_cliente());
        assert!(error.reintentable());
    }

    /// Una transición inválida entra envuelta en un error de repositorio y
    /// tiene que salir como 409 del cliente, no como 500 nuestro.
    #[test]
    fn una_transicion_invalida_no_se_reporta_como_fallo_interno() {
        use dp_dominio::EstadoJob;

        let dominio = ErrorRepositorio::Transicion(TransicionInvalida {
            desde: EstadoJob::Procesando,
            hacia: EstadoJob::Cancelado,
        });
        let api: ErrorApi = dominio.into();

        assert_eq!(api.estado_http(), StatusCode::CONFLICT);
        assert_eq!(api.codigo(), "E_TRANSICION_INVALIDA");
        assert!(api.culpa_del_cliente());
    }

    /// La clasificación del motor se hereda sin reinterpretar el mensaje.
    #[test]
    fn el_error_del_motor_conserva_su_codigo_y_su_culpa() {
        let del_cliente: ErrorApi = ErrorDp::ColumnaFaltante("email".to_string()).into();
        assert_eq!(del_cliente.codigo(), "E_COLUMNA_FALTANTE");
        assert_eq!(del_cliente.estado_http(), StatusCode::BAD_REQUEST);

        let interno: ErrorApi = ErrorDp::Escritura(std::io::Error::other("disco")).into();
        assert_eq!(interno.codigo(), "E_ESCRITURA");
        assert_eq!(interno.estado_http(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn el_limite_de_jobs_en_vuelo_responde_429() {
        let error = ErrorApi::DemasiadosJobs {
            en_vuelo: 1,
            limite: 1,
        };
        assert_eq!(error.estado_http(), StatusCode::TOO_MANY_REQUESTS);
        assert!(error.to_string().contains("1 jobs sin terminar"));
    }

    #[test]
    fn la_cadena_de_causas_llega_hasta_el_error_de_io() {
        let error = ErrorApi::Almacen(ErrorAlmacen::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "acceso denegado",
        )));
        let causas = error.causas();
        assert!(!causas.is_empty());
        assert!(causas[0].contains("acceso denegado"));
    }
}
