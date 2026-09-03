//! El job y su máquina de estados.
//!
//! Este es el corazón del sistema. Las transiciones son métodos que devuelven
//! `Result`, no asignaciones a un campo público, y eso no es ceremonia: hace
//! que completar un job cancelado sea **imposible** en vez de improbable.
//!
//! Fíjate en que `Job` no implementa `Serialize`. Es deliberado: tiene campos
//! como `reclamado_por` que son detalles internos de la cola y que jamás deben
//! salir en una respuesta de la API. La conversión al JSON público se hace en
//! `dp-api` con un tipo aparte, así que un campo nuevo aquí no se filtra solo.

use crate::error::TransicionInvalida;
use crate::ids::{IdArchivo, IdJob, IdOrganizacion};
use crate::opciones::OpcionesJob;
use crate::plan::Limites;
use chrono::{DateTime, Duration, Utc};
use dp_core::{ErrorDp, Resumen};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Cuántas veces se reintenta un job antes de darlo por perdido.
pub const MAX_INTENTOS: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EstadoJob {
    /// Créditos reservados, esperando que un worker lo tome.
    #[serde(rename = "queued")]
    Encolado,
    /// Un worker lo reclamó y tiene un plazo para terminarlo.
    #[serde(rename = "processing")]
    Procesando,
    #[serde(rename = "completed")]
    Completado,
    #[serde(rename = "failed")]
    Fallido,
    #[serde(rename = "cancelled")]
    Cancelado,
    /// Agotó los reintentos. Requiere intervención humana.
    #[serde(rename = "dead_letter")]
    SinSalida,
}

impl EstadoJob {
    pub fn es_final(&self) -> bool {
        matches!(
            self,
            EstadoJob::Completado
                | EstadoJob::Fallido
                | EstadoJob::Cancelado
                | EstadoJob::SinSalida
        )
    }

    pub fn etiqueta(&self) -> &'static str {
        match self {
            EstadoJob::Encolado => "queued",
            EstadoJob::Procesando => "processing",
            EstadoJob::Completado => "completed",
            EstadoJob::Fallido => "failed",
            EstadoJob::Cancelado => "cancelled",
            EstadoJob::SinSalida => "dead_letter",
        }
    }
}

impl fmt::Display for EstadoJob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.etiqueta())
    }
}

/// Un error ya clasificado, listo para guardar y para devolver al cliente.
///
/// Este sí lleva `Serialize`: viaja tal cual dentro de la respuesta de la API,
/// con los nombres del contrato en inglés.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorDelJob {
    #[serde(rename = "code")]
    pub codigo: String,
    #[serde(rename = "message")]
    pub mensaje: String,
    #[serde(rename = "client_error")]
    pub culpa_del_cliente: bool,
}

impl ErrorDelJob {
    /// La cosecha del hallazgo 3: el motor ya sabe si su error es culpa del
    /// cliente, así que el worker no tiene que adivinarlo leyendo el mensaje.
    pub fn desde_motor(error: &ErrorDp) -> Self {
        ErrorDelJob {
            codigo: error.codigo().to_string(),
            mensaje: error.to_string(),
            culpa_del_cliente: error.es_culpa_del_cliente(),
        }
    }

    /// Un fallo nuestro, reintentable.
    pub fn interno(codigo: &str, mensaje: impl Into<String>) -> Self {
        ErrorDelJob {
            codigo: codigo.to_string(),
            mensaje: mensaje.into(),
            culpa_del_cliente: false,
        }
    }

    /// Un fallo del cliente, definitivo.
    pub fn del_cliente(codigo: &str, mensaje: impl Into<String>) -> Self {
        ErrorDelJob {
            codigo: codigo.to_string(),
            mensaje: mensaje.into(),
            culpa_del_cliente: true,
        }
    }
}

/// Qué pasó con un job que falló. El worker necesita distinguirlo para saber
/// si devolver los créditos o dejarlos reservados para el próximo intento.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Desenlace {
    /// Vuelve a la cola. `intento` es el número del que acaba de fallar.
    Reencolado { intento: u32 },
    /// Error definitivo del cliente.
    Fallido,
    /// Se agotaron los reintentos.
    SinSalida,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub id: IdJob,
    pub organizacion: IdOrganizacion,

    /// El nombre de la operación, tal como llega en el payload: `csv.clean`.
    /// Es texto y no un enum a propósito: el catálogo vive en el trait
    /// `Operacion` de `dp-core`, y así añadir una operación no obliga a tocar
    /// el dominio.
    pub operacion: String,
    pub opciones: OpcionesJob,

    /// Los topes con los que se aceptó el job. Congelados aquí para que el
    /// worker no dependa del plan actual de la organización.
    pub limites: Limites,

    pub entrada: IdArchivo,
    pub salida: Option<IdArchivo>,

    pub estado: EstadoJob,
    pub progreso: u8,
    pub intentos: u32,
    pub max_intentos: u32,

    pub creditos_reservados: u64,
    pub creditos_cobrados: Option<u64>,

    pub resumen: Option<Resumen>,
    pub error: Option<ErrorDelJob>,

    pub creado_en: DateTime<Utc>,
    pub iniciado_en: Option<DateTime<Utc>>,
    pub terminado_en: Option<DateTime<Utc>>,

    /// Quién lo tiene ahora mismo. Interno: nunca sale en la API.
    pub reclamado_por: Option<String>,
    /// Hasta cuándo lo tiene. Si pasa esta hora sin terminar, otro worker lo
    /// puede rescatar. Es lo que evita que un worker muerto congele trabajo.
    pub reclamado_hasta: Option<DateTime<Utc>>,
}

impl Job {
    pub fn nuevo(
        organizacion: IdOrganizacion,
        operacion: impl Into<String>,
        entrada: IdArchivo,
        opciones: OpcionesJob,
        limites: Limites,
        creditos_reservados: u64,
        ahora: DateTime<Utc>,
    ) -> Self {
        Job {
            id: IdJob::nuevo(),
            organizacion,
            operacion: operacion.into(),
            opciones,
            limites,
            entrada,
            salida: None,
            estado: EstadoJob::Encolado,
            progreso: 0,
            intentos: 0,
            max_intentos: MAX_INTENTOS,
            creditos_reservados,
            creditos_cobrados: None,
            resumen: None,
            error: None,
            creado_en: ahora,
            iniciado_en: None,
            terminado_en: None,
            reclamado_por: None,
            reclamado_hasta: None,
        }
    }

    /// Un worker toma el job y se compromete a terminarlo antes de `plazo`.
    pub fn reclamar(
        &mut self,
        worker: &str,
        ahora: DateTime<Utc>,
        plazo: Duration,
    ) -> Result<(), TransicionInvalida> {
        self.exigir(EstadoJob::Encolado, EstadoJob::Procesando)?;

        self.estado = EstadoJob::Procesando;
        self.intentos += 1;
        self.iniciado_en = Some(ahora);
        self.reclamado_por = Some(worker.to_string());
        self.reclamado_hasta = Some(ahora + plazo);
        Ok(())
    }

    pub fn completar(
        &mut self,
        salida: Option<IdArchivo>,
        resumen: Resumen,
        creditos_cobrados: u64,
        ahora: DateTime<Utc>,
    ) -> Result<(), TransicionInvalida> {
        self.exigir(EstadoJob::Procesando, EstadoJob::Completado)?;

        self.estado = EstadoJob::Completado;
        self.progreso = 100;
        self.salida = salida;
        self.resumen = Some(resumen);
        self.creditos_cobrados = Some(creditos_cobrados);
        self.terminado_en = Some(ahora);
        self.soltar();
        Ok(())
    }

    /// El job falló. El desenlace lo decide la clasificación del error, no
    /// quien llama.
    pub fn fallar(
        &mut self,
        error: ErrorDelJob,
        ahora: DateTime<Utc>,
    ) -> Result<Desenlace, TransicionInvalida> {
        self.exigir(EstadoJob::Procesando, EstadoJob::Fallido)?;
        Ok(self.resolver_fallo(error, ahora))
    }

    /// El cliente cancela. Solo se puede antes de que un worker lo tome:
    /// interrumpir un job en vuelo requiere cooperación del worker, y eso es
    /// trabajo de una fase posterior.
    pub fn cancelar(&mut self, ahora: DateTime<Utc>) -> Result<(), TransicionInvalida> {
        self.exigir(EstadoJob::Encolado, EstadoJob::Cancelado)?;

        self.estado = EstadoJob::Cancelado;
        self.terminado_en = Some(ahora);
        Ok(())
    }

    /// ¿Se le pasó el plazo a quien lo tiene?
    pub fn plazo_vencido(&self, ahora: DateTime<Utc>) -> bool {
        self.estado == EstadoJob::Procesando
            && self.reclamado_hasta.is_some_and(|limite| ahora >= limite)
    }

    /// Recupera un job cuyo worker desapareció.
    ///
    /// Devuelve `None` si no había nada que rescatar. Un worker que muere no
    /// puede reportar su propio error, así que lo sintetizamos como fallo
    /// interno: reintentable, porque la causa fue nuestra infraestructura.
    pub fn rescatar(&mut self, ahora: DateTime<Utc>) -> Option<Desenlace> {
        if !self.plazo_vencido(ahora) {
            return None;
        }

        let error = ErrorDelJob::interno(
            "E_WORKER_PERDIDO",
            format!(
                "el worker '{}' no reporto resultado antes de su plazo",
                self.reclamado_por.as_deref().unwrap_or("desconocido")
            ),
        );
        Some(self.resolver_fallo(error, ahora))
    }

    /// La decisión de reintentar, en un solo lugar para que un fallo reportado
    /// por el worker y un plazo vencido se traten exactamente igual.
    fn resolver_fallo(&mut self, error: ErrorDelJob, ahora: DateTime<Utc>) -> Desenlace {
        let culpa_del_cliente = error.culpa_del_cliente;
        self.error = Some(error);

        if culpa_del_cliente {
            // Reintentar un CSV al que le falta una columna daría el mismo
            // error las tres veces, gastando CPU que pagamos nosotros.
            self.estado = EstadoJob::Fallido;
            self.terminado_en = Some(ahora);
            self.soltar();
            Desenlace::Fallido
        } else if self.intentos < self.max_intentos {
            self.estado = EstadoJob::Encolado;
            self.progreso = 0;
            self.soltar();
            Desenlace::Reencolado {
                intento: self.intentos,
            }
        } else {
            self.estado = EstadoJob::SinSalida;
            self.terminado_en = Some(ahora);
            self.soltar();
            Desenlace::SinSalida
        }
    }

    /// Libera la reclamación. Un job que no está en PROCESANDO no debe
    /// aparecer como reclamado por nadie.
    fn soltar(&mut self) {
        self.reclamado_por = None;
        self.reclamado_hasta = None;
    }

    fn exigir(&self, desde: EstadoJob, hacia: EstadoJob) -> Result<(), TransicionInvalida> {
        if self.estado == desde {
            Ok(())
        } else {
            Err(TransicionInvalida {
                desde: self.estado,
                hacia,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job_de_prueba() -> Job {
        Job::nuevo(
            IdOrganizacion::nuevo(),
            "csv.clean",
            IdArchivo::nuevo(),
            OpcionesJob::default(),
            crate::plan::Plan::Pro.limites(),
            10,
            Utc::now(),
        )
    }

    fn plazo() -> Duration {
        Duration::seconds(300)
    }

    #[test]
    fn un_job_nace_encolado_y_sin_intentos() {
        let job = job_de_prueba();
        assert_eq!(job.estado, EstadoJob::Encolado);
        assert_eq!(job.intentos, 0);
        assert_eq!(job.progreso, 0);
        assert!(job.reclamado_por.is_none());
        assert!(!job.estado.es_final());
    }

    #[test]
    fn el_camino_feliz_lleva_a_completado() {
        let mut job = job_de_prueba();
        let ahora = Utc::now();

        job.reclamar("worker-1", ahora, plazo()).unwrap();
        assert_eq!(job.estado, EstadoJob::Procesando);
        assert_eq!(job.intentos, 1);
        assert_eq!(job.reclamado_por.as_deref(), Some("worker-1"));

        job.completar(Some(IdArchivo::nuevo()), Resumen::default(), 10, ahora)
            .unwrap();

        assert_eq!(job.estado, EstadoJob::Completado);
        assert_eq!(job.progreso, 100);
        assert_eq!(job.creditos_cobrados, Some(10));
        assert!(job.salida.is_some());
        // Ya no está reclamado por nadie.
        assert!(job.reclamado_por.is_none());
    }

    /// Dos workers no pueden tomar el mismo job: el segundo `reclamar` falla
    /// porque el estado ya no es ENCOLADO.
    #[test]
    fn un_job_no_se_puede_reclamar_dos_veces() {
        let mut job = job_de_prueba();
        let ahora = Utc::now();

        job.reclamar("worker-1", ahora, plazo()).unwrap();
        let error = job
            .reclamar("worker-2", ahora, plazo())
            .expect_err("el segundo worker no deberia poder tomarlo");

        assert_eq!(error.desde, EstadoJob::Procesando);
        assert_eq!(job.reclamado_por.as_deref(), Some("worker-1"));
    }

    /// El invariante que justifica todo el módulo.
    #[test]
    fn un_job_cancelado_no_se_puede_completar() {
        let mut job = job_de_prueba();
        let ahora = Utc::now();

        job.cancelar(ahora).unwrap();
        let error = job
            .completar(None, Resumen::default(), 0, ahora)
            .expect_err("no deberia poder completarse");

        assert_eq!(error.desde, EstadoJob::Cancelado);
        assert_eq!(job.estado, EstadoJob::Cancelado);
    }

    #[test]
    fn un_job_en_proceso_ya_no_se_puede_cancelar() {
        let mut job = job_de_prueba();
        let ahora = Utc::now();

        job.reclamar("worker-1", ahora, plazo()).unwrap();
        assert!(job.cancelar(ahora).is_err());
    }

    /// Un error del cliente no se reintenta ni una vez.
    #[test]
    fn un_error_del_cliente_falla_de_inmediato() {
        let mut job = job_de_prueba();
        let ahora = Utc::now();

        job.reclamar("worker-1", ahora, plazo()).unwrap();
        let desenlace = job
            .fallar(
                ErrorDelJob::del_cliente("E_COLUMNA_FALTANTE", "falta 'email'"),
                ahora,
            )
            .unwrap();

        assert_eq!(desenlace, Desenlace::Fallido);
        assert_eq!(job.estado, EstadoJob::Fallido);
        assert_eq!(job.intentos, 1);
        assert!(job.estado.es_final());
    }

    /// Un fallo interno vuelve a la cola hasta agotar los intentos, y entonces
    /// cae en SIN_SALIDA en vez de reintentarse para siempre.
    #[test]
    fn un_fallo_interno_se_reintenta_hasta_agotarse() {
        let mut job = job_de_prueba();
        let ahora = Utc::now();

        for intento in 1..MAX_INTENTOS {
            job.reclamar("worker-1", ahora, plazo()).unwrap();
            let desenlace = job
                .fallar(ErrorDelJob::interno("E_ESCRITURA", "disco lleno"), ahora)
                .unwrap();

            assert_eq!(desenlace, Desenlace::Reencolado { intento });
            assert_eq!(job.estado, EstadoJob::Encolado);
            assert!(job.reclamado_por.is_none());
        }

        // El último intento agota el presupuesto.
        job.reclamar("worker-1", ahora, plazo()).unwrap();
        let desenlace = job
            .fallar(ErrorDelJob::interno("E_ESCRITURA", "disco lleno"), ahora)
            .unwrap();

        assert_eq!(desenlace, Desenlace::SinSalida);
        assert_eq!(job.estado, EstadoJob::SinSalida);
        assert_eq!(job.intentos, MAX_INTENTOS);
    }

    /// El escenario de resiliencia que el PDF pide probar en la sección 11:
    /// un worker muere a media ejecución y el trabajo no se pierde.
    #[test]
    fn un_worker_muerto_no_congela_el_job() {
        let mut job = job_de_prueba();
        let inicio = Utc::now();

        job.reclamar("worker-que-muere", inicio, Duration::seconds(60))
            .unwrap();

        // Antes del plazo, nadie lo toca.
        assert!(!job.plazo_vencido(inicio + Duration::seconds(59)));
        assert!(job.rescatar(inicio + Duration::seconds(59)).is_none());

        // Pasado el plazo, se rescata y vuelve a la cola.
        let despues = inicio + Duration::seconds(61);
        assert!(job.plazo_vencido(despues));

        let desenlace = job.rescatar(despues).expect("deberia rescatarse");
        assert_eq!(desenlace, Desenlace::Reencolado { intento: 1 });
        assert_eq!(job.estado, EstadoJob::Encolado);
        assert_eq!(job.error.as_ref().unwrap().codigo, "E_WORKER_PERDIDO");
        // Y el error del rescate es interno, así que sí se reintenta.
        assert!(!job.error.as_ref().unwrap().culpa_del_cliente);
    }

    #[test]
    fn un_job_completado_no_se_rescata() {
        let mut job = job_de_prueba();
        let ahora = Utc::now();

        job.reclamar("worker-1", ahora, Duration::seconds(1)).unwrap();
        job.completar(None, Resumen::default(), 1, ahora).unwrap();

        assert!(job.rescatar(ahora + Duration::hours(1)).is_none());
        assert_eq!(job.estado, EstadoJob::Completado);
    }

    /// El error del motor se traduce sin que nadie interprete su mensaje.
    #[test]
    fn el_error_del_motor_llega_ya_clasificado() {
        let del_cliente = ErrorDp::ColumnaFaltante("email".to_string());
        let convertido = ErrorDelJob::desde_motor(&del_cliente);
        assert_eq!(convertido.codigo, "E_COLUMNA_FALTANTE");
        assert!(convertido.culpa_del_cliente);

        let interno = ErrorDp::Escritura(std::io::Error::other("disco lleno"));
        let convertido = ErrorDelJob::desde_motor(&interno);
        assert_eq!(convertido.codigo, "E_ESCRITURA");
        assert!(!convertido.culpa_del_cliente);
    }

    #[test]
    fn el_estado_se_serializa_con_los_nombres_del_pdf() {
        assert_eq!(
            serde_json::to_string(&EstadoJob::Encolado).unwrap(),
            "\"queued\""
        );
        assert_eq!(
            serde_json::to_string(&EstadoJob::Completado).unwrap(),
            "\"completed\""
        );
        assert_eq!(
            serde_json::to_string(&EstadoJob::SinSalida).unwrap(),
            "\"dead_letter\""
        );
    }
}
