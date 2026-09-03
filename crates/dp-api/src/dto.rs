//! Los tipos que viajan por el cable.
//!
//! Existen separados de los tipos del dominio por una razón concreta y no por
//! purismo: `Job` tiene un campo `reclamado_por` con el nombre del worker que
//! lo está procesando. Si la API serializara el `Job` directamente, ese dato
//! interno saldría en la respuesta. Y lo peor no es este campo, es el
//! siguiente que alguien añada al dominio sin pensar en la API.
//!
//! Con esta separación, exponer un campo nuevo es un acto deliberado: hay que
//! escribirlo aquí.

use chrono::{DateTime, Utc};
use dp_core::Resumen;
use dp_dominio::{
    Archivo, ErrorDelJob, EstadoArchivo, EstadoJob, IdArchivo, IdJob, Job, TipoArchivo,
};
use serde::{Deserialize, Serialize};

/// `POST /v1/jobs`, tal como lo describe la sección 15 del PDF.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeticionCrearJob {
    pub operation: String,
    pub file_id: IdArchivo,
    #[serde(default)]
    pub options: dp_dominio::OpcionesJob,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeticionPresign {
    /// Solo para mostrar. No se usa para construir ninguna ruta.
    pub filename: String,
}

#[derive(Debug, Serialize)]
pub struct RespuestaPresign {
    pub file_id: IdArchivo,
    pub upload_url: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct RespuestaDescarga {
    pub file_id: IdArchivo,
    pub download_url: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct RespuestaArchivo {
    pub file_id: IdArchivo,
    pub filename: String,
    pub status: EstadoArchivo,
    #[serde(rename = "type")]
    pub tipo: TipoArchivo,
    pub bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl From<&Archivo> for RespuestaArchivo {
    fn from(archivo: &Archivo) -> Self {
        RespuestaArchivo {
            file_id: archivo.id,
            filename: archivo.nombre_original.clone(),
            status: archivo.estado,
            tipo: archivo.tipo,
            bytes: archivo.bytes,
            sha256: archivo.sha256.clone(),
            created_at: archivo.creado_en,
            expires_at: archivo.vence_en,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RespuestaJob {
    pub job_id: IdJob,
    pub status: EstadoJob,
    pub operation: String,
    pub progress: u8,
    pub attempts: u32,
    pub max_attempts: u32,
    pub input_file_id: IdArchivo,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_file_id: Option<IdArchivo>,

    /// El resumen que produce el motor, sin transformar. Es el mismo objeto
    /// que la CLI imprime, así que lo que el cliente ve en la API y lo que
    /// vemos nosotros depurando es idéntico.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Resumen>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorDelJob>,

    pub credits_reserved: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credits_charged: Option<u64>,

    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    // `reclamado_por` y `reclamado_hasta` NO están aquí, y es a propósito:
    // son detalles de la cola. Al cliente no le importa qué worker le tocó.
}

impl From<&Job> for RespuestaJob {
    fn from(job: &Job) -> Self {
        RespuestaJob {
            job_id: job.id,
            status: job.estado,
            operation: job.operacion.clone(),
            progress: job.progreso,
            attempts: job.intentos,
            max_attempts: job.max_intentos,
            input_file_id: job.entrada,
            output_file_id: job.salida,
            result: job.resumen,
            error: job.error.clone(),
            credits_reserved: job.creditos_reservados,
            credits_charged: job.creditos_cobrados,
            created_at: job.creado_en,
            started_at: job.iniciado_en,
            finished_at: job.terminado_en,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RespuestaLista<T> {
    pub data: Vec<T>,
    pub count: usize,
}

#[derive(Debug, Serialize)]
pub struct RespuestaOperacion {
    pub name: &'static str,
    pub description: &'static str,
    pub produces_file: bool,
    /// Cuánto cuesta la unidad mínima. La sección 7 del PDF pide que el
    /// catálogo diga su precio: un cliente tiene que poder estimar el costo
    /// antes de mandar el job.
    pub minimum_credits: u64,
}

/// Parámetros de las URLs firmadas.
///
/// La organización viaja en la URL porque estos dos endpoints no llevan
/// cabecera de autorización: la firma **es** la credencial. Y como la
/// organización forma parte de lo firmado, no se puede cambiar para alcanzar
/// el archivo de otro cliente. Es el mismo esquema de una URL prefirmada de
/// S3, donde el bucket y la clave van dentro de lo que se firma.
#[derive(Debug, Deserialize)]
pub struct ParametrosFirma {
    pub org: dp_dominio::IdOrganizacion,
    pub exp: i64,
    pub sig: String,
    /// Solo en las URLs de subida: el tope de bytes que autorizó el plan.
    /// Va firmado, así que inflarlo invalida la firma.
    #[serde(default)]
    pub max: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct FiltroJobs {
    #[serde(default)]
    pub status: Option<EstadoJob>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use dp_dominio::{IdOrganizacion, OpcionesJob, Plan};

    fn job_de_prueba() -> Job {
        Job::nuevo(
            IdOrganizacion::nuevo(),
            "csv.clean",
            IdArchivo::nuevo(),
            OpcionesJob::default(),
            Plan::Pro.limites(),
            5,
            Utc::now(),
        )
    }

    /// El test que justifica todo el módulo: el nombre del worker no puede
    /// salir en la respuesta.
    #[test]
    fn la_respuesta_no_expone_datos_internos_de_la_cola() {
        let mut job = job_de_prueba();
        job.reclamar("worker-secreto-01", Utc::now(), chrono::Duration::seconds(60))
            .unwrap();

        let json = serde_json::to_string(&RespuestaJob::from(&job)).unwrap();

        assert!(!json.contains("worker-secreto-01"));
        assert!(!json.contains("reclamado"));
        assert!(!json.contains("claimed"));
        // Y sí lleva lo que el cliente necesita.
        assert!(json.contains("\"status\":\"processing\""));
        assert!(json.contains("\"attempts\":1"));
    }

    #[test]
    fn un_job_recien_creado_omite_los_campos_vacios() {
        let job = Job::nuevo(
            IdOrganizacion::nuevo(),
            "csv.clean",
            IdArchivo::nuevo(),
            OpcionesJob::default(),
            Plan::Pro.limites(),
            5,
            Utc::now(),
        );
        let json = serde_json::to_string(&RespuestaJob::from(&job)).unwrap();

        // Sin resultado ni error todavía: los campos no salen como null.
        assert!(!json.contains("\"result\""));
        assert!(!json.contains("\"error\""));
        assert!(!json.contains("\"output_file_id\""));
        assert!(json.contains("\"status\":\"queued\""));
    }

    #[test]
    fn el_resumen_del_motor_sale_tal_cual() {
        let mut job = Job::nuevo(
            IdOrganizacion::nuevo(),
            "csv.clean",
            IdArchivo::nuevo(),
            OpcionesJob::default(),
            Plan::Pro.limites(),
            5,
            Utc::now(),
        );
        let ahora = Utc::now();
        job.reclamar("w1", ahora, chrono::Duration::seconds(60))
            .unwrap();
        job.completar(
            Some(IdArchivo::nuevo()),
            Resumen {
                leidas: 15,
                escritas: 12,
                duplicadas: 3,
                ..Resumen::default()
            },
            10,
            ahora,
        )
        .unwrap();

        let json = serde_json::to_value(RespuestaJob::from(&job)).unwrap();
        assert_eq!(json["result"]["leidas"], 15);
        assert_eq!(json["result"]["duplicadas"], 3);
        assert_eq!(json["credits_charged"], 10);
        assert_eq!(json["progress"], 100);
    }

    #[test]
    fn la_peticion_de_job_usa_el_formato_del_pdf() {
        let json = r#"{
            "operation": "csv.clean",
            "file_id": "file_0123456789abcdef0123456789abcdef",
            "options": { "remove_duplicates": true }
        }"#;
        let peticion: PeticionCrearJob = serde_json::from_str(json).unwrap();

        assert_eq!(peticion.operation, "csv.clean");
        assert_eq!(peticion.options.deduplicar, Some(true));
    }

    #[test]
    fn un_campo_desconocido_en_la_peticion_se_rechaza() {
        let json = r#"{"operation":"csv.clean","file_id":"file_0123456789abcdef0123456789abcdef","operacion":"x"}"#;
        assert!(serde_json::from_str::<PeticionCrearJob>(json).is_err());
    }
}
