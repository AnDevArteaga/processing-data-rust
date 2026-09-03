//! Las opciones que el cliente manda en `POST /v1/jobs`.
//!
//! Aquí se ve una separación que vale la pena entender: los identificadores de
//! Rust están en español, como todo el código, pero el contrato JSON está en
//! inglés porque así lo definen los ejemplos de la sección 15 del PDF y porque
//! es lo que espera cualquiera que integre una API. `#[serde(rename)]` existe
//! exactamente para eso: el nombre en el cable y el nombre en el código son
//! decisiones independientes.

use crate::plan::Limites;
use dp_core::Config;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FormatoSalida {
    #[default]
    Csv,
    Json,
}

impl FormatoSalida {
    pub fn extension(&self) -> &'static str {
        match self {
            FormatoSalida::Csv => "csv",
            FormatoSalida::Json => "json",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
// `default` deja que el cliente omita cualquier campo.
// `deny_unknown_fields` es lo importante: si alguien escribe "remove_duplicate"
// sin la s, la petición falla con un mensaje claro en vez de aceptarse y no
// hacer nada. Un job que silenciosamente ignora lo que le pediste es peor que
// un error.
#[serde(default, deny_unknown_fields)]
pub struct OpcionesJob {
    /// Columnas que forman la clave de deduplicación.
    #[serde(rename = "dedup_keys")]
    pub claves: Option<Vec<String>>,

    /// Columnas a normalizar y validar como email.
    #[serde(rename = "email_columns")]
    pub emails: Option<Vec<String>>,

    /// Columnas a normalizar como teléfono.
    #[serde(rename = "phone_columns")]
    pub telefonos: Option<Vec<String>>,

    /// Atajo para desactivar la deduplicación sin listar columnas.
    #[serde(rename = "remove_duplicates")]
    pub deduplicar: Option<bool>,

    #[serde(rename = "output_format")]
    pub formato: FormatoSalida,
}

impl OpcionesJob {
    /// Traduce las opciones del cliente a la configuración del motor,
    /// imponiendo los límites del plan.
    ///
    /// El detalle que importa: `limite_claves` NO es una opción del cliente.
    /// Se toma del plan y se escribe al final, después de todo lo demás, para
    /// que ninguna combinación de opciones pueda subirlo. Si fuera un campo de
    /// `OpcionesJob`, cualquiera podría pedir mil millones y tumbar el worker.
    pub fn a_config(&self, limites: &Limites) -> Config {
        let mut config = Config::default();

        if let Some(claves) = &self.claves {
            config.claves = claves.clone();
        }
        if let Some(emails) = &self.emails {
            config.emails = emails.clone();
        }
        if let Some(telefonos) = &self.telefonos {
            config.telefonos = telefonos.clone();
        }
        if self.deduplicar == Some(false) {
            config.claves.clear();
        }

        config.limite_claves = limites.claves_dedup;
        config
    }

    pub fn deduplica(&self) -> bool {
        if self.deduplicar == Some(false) {
            return false;
        }
        match &self.claves {
            Some(claves) => !claves.is_empty(),
            None => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::Plan;

    #[test]
    fn sin_opciones_se_usa_la_configuracion_por_defecto_del_motor() {
        let config = OpcionesJob::default().a_config(&Plan::Pro.limites());
        assert_eq!(config.claves, vec!["email".to_string()]);
        assert_eq!(config.emails, vec!["email".to_string()]);
    }

    #[test]
    fn las_claves_del_cliente_reemplazan_las_por_defecto() {
        let opciones = OpcionesJob {
            claves: Some(vec!["nit".to_string(), "ciudad".to_string()]),
            ..Default::default()
        };
        let config = opciones.a_config(&Plan::Pro.limites());
        assert_eq!(config.claves, vec!["nit", "ciudad"]);
    }

    #[test]
    fn remove_duplicates_en_falso_apaga_la_deduplicacion() {
        let opciones = OpcionesJob {
            deduplicar: Some(false),
            claves: Some(vec!["email".to_string()]),
            ..Default::default()
        };
        assert!(opciones.a_config(&Plan::Pro.limites()).claves.is_empty());
        assert!(!opciones.deduplica());
    }

    /// El invariante de seguridad de este módulo: el límite lo pone el plan,
    /// siempre, y no hay opción del cliente que lo cambie.
    #[test]
    fn el_limite_de_claves_lo_impone_el_plan() {
        let opciones = OpcionesJob {
            claves: Some(vec!["email".to_string()]),
            ..Default::default()
        };

        let free = opciones.a_config(&Plan::Free.limites());
        let business = opciones.a_config(&Plan::Business.limites());

        assert_eq!(free.limite_claves, 50_000);
        assert_eq!(business.limite_claves, 12_000_000);
    }

    #[test]
    fn el_contrato_json_usa_los_nombres_del_pdf() {
        let json =
            r#"{"remove_duplicates": true, "dedup_keys": ["email"], "output_format": "json"}"#;
        let opciones: OpcionesJob = serde_json::from_str(json).unwrap();

        assert_eq!(opciones.deduplicar, Some(true));
        assert_eq!(opciones.claves, Some(vec!["email".to_string()]));
        assert_eq!(opciones.formato, FormatoSalida::Json);
    }

    /// Una opción mal escrita tiene que doler, no pasar desapercibida.
    #[test]
    fn una_opcion_desconocida_se_rechaza() {
        let json = r#"{"remove_duplicate": true}"#;
        let error = serde_json::from_str::<OpcionesJob>(json)
            .expect_err("deberia rechazar el campo mal escrito");
        assert!(error.to_string().contains("remove_duplicate"));
    }

    #[test]
    fn un_json_vacio_es_valido() {
        let opciones: OpcionesJob = serde_json::from_str("{}").unwrap();
        assert_eq!(opciones, OpcionesJob::default());
        assert_eq!(opciones.formato, FormatoSalida::Csv);
    }
}
