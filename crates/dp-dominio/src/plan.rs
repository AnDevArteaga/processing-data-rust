//! Planes y sus límites.
//!
//! Los planes son código, no filas en una tabla. Es una decisión consciente:
//! mientras solo tú los cambies, un `match` te da revisión de código,
//! historial en git y un despliegue que se puede revertir. El día que un
//! comercial necesite crear un plan sin desplegar, esto se muda a la base de
//! datos, y no antes.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Plan {
    Free,
    Starter,
    Pro,
    Business,
}

/// Los topes que la API valida antes de encolar y que el worker aplica al
/// ejecutar. Sección 9 del PDF: "limitar CPU/memoria/tiempo por job".
///
/// Se guardan **dentro del job**. Podría parecer redundante (el plan está en
/// la organización), pero tiene dos consecuencias que valen la duplicación: el
/// worker no necesita saber nada de planes ni consultar la organización, y un
/// cambio de plan a mitad de vuelo no altera un job ya encolado. El job se
/// ejecuta con las reglas que tenía cuando se aceptó.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Limites {
    /// Tamaño máximo de un archivo subido.
    pub bytes_por_archivo: u64,
    /// Tope del índice de deduplicación. Es el que ya implementamos en el
    /// motor: acota la memoria del worker.
    pub claves_dedup: usize,
    /// Un job que se pasa de aquí se aborta. Evita que un archivo patológico
    /// ocupe un worker indefinidamente.
    pub segundos_por_job: u64,
    /// Cuántos jobs puede tener una organización sin terminar a la vez. Es lo
    /// que evita que un cliente monopolice todos los workers.
    pub jobs_en_vuelo: usize,
    /// Créditos incluidos en el periodo.
    pub creditos_mensuales: u64,
}

impl Plan {
    /// Sección 7.1 del PDF. Los valores de créditos y tamaños salen de ahí;
    /// el tope de claves de deduplicación lo derivé de nuestras mediciones:
    /// un millón de claves cuesta unos 85 MB de memoria en el worker.
    pub fn limites(&self) -> Limites {
        match self {
            Plan::Free => Limites {
                bytes_por_archivo: 5 * 1024 * 1024,
                claves_dedup: 50_000,
                segundos_por_job: 30,
                jobs_en_vuelo: 1,
                creditos_mensuales: 100,
            },
            Plan::Starter => Limites {
                bytes_por_archivo: 50 * 1024 * 1024,
                claves_dedup: 1_000_000,
                segundos_por_job: 300,
                jobs_en_vuelo: 3,
                creditos_mensuales: 5_000,
            },
            Plan::Pro => Limites {
                bytes_por_archivo: 250 * 1024 * 1024,
                claves_dedup: 5_000_000,
                segundos_por_job: 900,
                jobs_en_vuelo: 10,
                creditos_mensuales: 30_000,
            },
            Plan::Business => Limites {
                bytes_por_archivo: 1024 * 1024 * 1024,
                claves_dedup: 12_000_000,
                segundos_por_job: 3_600,
                jobs_en_vuelo: 30,
                creditos_mensuales: 150_000,
            },
        }
    }

    pub fn etiqueta(&self) -> &'static str {
        match self {
            Plan::Free => "free",
            Plan::Starter => "starter",
            Plan::Pro => "pro",
            Plan::Business => "business",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un plan superior nunca debe ser peor en ninguna dimensión. Suena obvio,
    /// y es justo el tipo de cosa que se rompe al ajustar un número a mano.
    #[test]
    fn los_limites_crecen_de_forma_monotona() {
        let escalera = [Plan::Free, Plan::Starter, Plan::Pro, Plan::Business];

        for par in escalera.windows(2) {
            let menor = par[0].limites();
            let mayor = par[1].limites();

            assert!(
                mayor.bytes_por_archivo > menor.bytes_por_archivo,
                "{:?} deberia permitir archivos mas grandes que {:?}",
                par[1],
                par[0]
            );
            assert!(mayor.claves_dedup > menor.claves_dedup);
            assert!(mayor.segundos_por_job > menor.segundos_por_job);
            assert!(mayor.jobs_en_vuelo > menor.jobs_en_vuelo);
            assert!(mayor.creditos_mensuales > menor.creditos_mensuales);
        }
    }

    /// El plan gratuito tiene que ser barato de servir: es el que va a recibir
    /// todo el abuso.
    #[test]
    fn el_plan_gratuito_no_puede_tumbar_un_worker() {
        let free = Plan::Free.limites();
        assert_eq!(free.jobs_en_vuelo, 1);
        // 50.000 claves son unos 4 MB de indice: irrelevante.
        assert!(free.claves_dedup <= 50_000);
        assert!(free.segundos_por_job <= 60);
    }
}
