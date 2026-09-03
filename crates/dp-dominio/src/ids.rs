//! Identificadores tipados.
//!
//! La alternativa perezosa sería usar `Uuid` en todas partes. El problema es
//! que entonces esta firma compila:
//!
//! ```text
//! fn obtener_archivo(organizacion: Uuid, archivo: Uuid)
//! obtener_archivo(id_archivo, id_organizacion)  // argumentos invertidos
//! ```
//!
//! Con tipos distintos ese error no llega ni a ejecutarse. Y como cada
//! identificador se muestra con su prefijo (`job_a1b2…`, `file_c3d4…`), un
//! identificador en un log o en un ticket de soporte dice por sí solo de qué
//! es.

use std::fmt;
use std::str::FromStr;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error, PartialEq)]
pub enum ErrorId {
    #[error("el identificador '{recibido}' no empieza por '{esperado}_'")]
    PrefijoIncorrecto {
        recibido: String,
        esperado: &'static str,
    },

    #[error("el identificador '{0}' no contiene un UUID valido")]
    UuidInvalido(String),
}

/// Genera un tipo de identificador completo.
///
/// `macro_rules!` es una macro declarativa: reescribe código antes de
/// compilar. `$nombre:ident` captura un identificador y `$prefijo:literal` un
/// literal. Sin esto, los cuatro tipos de abajo serían el mismo bloque de
/// cincuenta líneas copiado cuatro veces, y el quinto lo copiaríamos mal.
macro_rules! identificador {
    ($nombre:ident, $prefijo:literal, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $nombre(Uuid);

        impl $nombre {
            pub const PREFIJO: &'static str = $prefijo;

            /// Genera uno nuevo al azar.
            pub fn nuevo() -> Self {
                Self(Uuid::new_v4())
            }

            pub fn uuid(&self) -> Uuid {
                self.0
            }

            pub fn desde_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }
        }

        impl fmt::Display for $nombre {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                // `simple()` imprime el UUID sin guiones: mas corto y sin
                // caracteres que haya que escapar en una URL.
                write!(f, "{}_{}", $prefijo, self.0.simple())
            }
        }

        impl FromStr for $nombre {
            type Err = ErrorId;

            fn from_str(texto: &str) -> Result<Self, Self::Err> {
                let sin_prefijo = texto.strip_prefix(concat!($prefijo, "_")).ok_or_else(|| {
                    ErrorId::PrefijoIncorrecto {
                        recibido: texto.to_string(),
                        esperado: $prefijo,
                    }
                })?;

                Uuid::parse_str(sin_prefijo)
                    .map(Self)
                    .map_err(|_| ErrorId::UuidInvalido(texto.to_string()))
            }
        }

        // Serializamos a mano y no con `#[derive(Serialize)]` porque el derive
        // sobre un newtype produciria el UUID desnudo. Queremos que el JSON de
        // la API diga "job_a1b2..." tal como muestra la seccion 15 del PDF.
        impl serde::Serialize for $nombre {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                // `collect_str` usa nuestro Display sin construir un String.
                s.collect_str(self)
            }
        }

        impl<'de> serde::Deserialize<'de> for $nombre {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let texto = String::deserialize(d)?;
                texto.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}

identificador!(
    IdOrganizacion,
    "org",
    "La organizacion es la unidad de facturacion y la frontera de aislamiento."
);
identificador!(IdArchivo, "file", "Un archivo subido o producido.");
identificador!(IdJob, "job", "Una unidad de trabajo asincrona.");
identificador!(IdApiKey, "key", "Una credencial de acceso programatico.");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn un_id_se_muestra_con_su_prefijo() {
        let id = IdJob::nuevo();
        let texto = id.to_string();
        assert!(texto.starts_with("job_"));
        // 4 del prefijo mas el guion bajo mas 32 del UUID sin guiones.
        assert_eq!(texto.len(), 4 + 32);
    }

    #[test]
    fn un_id_sobrevive_ida_y_vuelta_a_texto() {
        let id = IdArchivo::nuevo();
        let recuperado: IdArchivo = id.to_string().parse().unwrap();
        assert_eq!(id, recuperado);
    }

    /// La razón de ser de todo este módulo: no se puede colar el
    /// identificador de un archivo donde se espera el de un job.
    #[test]
    fn el_prefijo_de_otro_tipo_se_rechaza() {
        let archivo = IdArchivo::nuevo().to_string();
        let error = archivo.parse::<IdJob>().expect_err("no deberia aceptarlo");
        assert!(matches!(error, ErrorId::PrefijoIncorrecto { .. }));
    }

    #[test]
    fn un_uuid_desnudo_se_rechaza() {
        let desnudo = Uuid::new_v4().to_string();
        assert!(desnudo.parse::<IdJob>().is_err());
    }

    #[test]
    fn un_uuid_mal_formado_se_rechaza() {
        let error = "job_noesunuuid"
            .parse::<IdJob>()
            .expect_err("no deberia aceptarlo");
        assert!(matches!(error, ErrorId::UuidInvalido(_)));
    }

    #[test]
    fn el_json_lleva_el_prefijo_y_no_el_uuid_desnudo() {
        let id = IdJob::nuevo();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{id}\""));

        let recuperado: IdJob = serde_json::from_str(&json).unwrap();
        assert_eq!(id, recuperado);
    }

    #[test]
    fn un_json_con_prefijo_ajeno_falla_al_deserializar() {
        let json = format!("\"{}\"", IdOrganizacion::nuevo());
        assert!(serde_json::from_str::<IdJob>(&json).is_err());
    }
}
