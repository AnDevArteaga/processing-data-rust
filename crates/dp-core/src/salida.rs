//! El contrato de salida del motor.
//!
//! Vive en la librería y no en la CLI a propósito: el worker que consuma la
//! cola de Redis va a producir exactamente esta misma envoltura, y la API la
//! va a devolver al cliente sin transformarla.

use crate::error::ErrorDp;
use crate::modelo::Resumen;
use serde::Serialize;

/// Resultado de un job en formato legible por máquina.
///
/// `#[serde(skip_serializing_if = "Option::is_none")]` omite el campo del JSON
/// cuando vale None, en vez de escribir `"error": null`. Así el consumidor ve
/// solo el campo que aplica.
#[derive(Debug, Serialize)]
pub struct Envoltura {
    pub ok: bool,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub resumen: Option<Resumen>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<DetalleError>,
}

#[derive(Debug, Serialize)]
pub struct DetalleError {
    /// Código estable. Esto es lo que se agrupa en las alertas, nunca el mensaje.
    pub codigo: &'static str,
    /// Si el orquestador debe volver a encolar el job.
    pub reintentable: bool,
    /// Si el fallo es por datos del cliente (equivale a un 4xx).
    pub culpa_del_cliente: bool,
    /// Texto para humanos. Puede cambiar entre versiones sin romper a nadie.
    pub mensaje: String,
    /// Errores subyacentes, del más externo al más interno.
    pub causas: Vec<String>,
}

impl Envoltura {
    pub fn exito(resumen: Resumen) -> Self {
        Envoltura {
            ok: true,
            resumen: Some(resumen),
            error: None,
        }
    }

    pub fn fallo(error: &ErrorDp) -> Self {
        Envoltura {
            ok: false,
            resumen: None,
            error: Some(DetalleError {
                codigo: error.codigo(),
                reintentable: error.es_reintentable(),
                culpa_del_cliente: error.es_culpa_del_cliente(),
                mensaje: error.to_string(),
                causas: error.cadena_de_causas(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_exito_no_incluye_el_campo_error() {
        let json = serde_json::to_string(&Envoltura::exito(Resumen::default())).unwrap();
        assert!(json.contains("\"ok\":true"));
        assert!(json.contains("\"resumen\""));
        // El campo error se omite por completo, no sale como null.
        assert!(!json.contains("error"));
    }

    #[test]
    fn el_fallo_expone_codigo_y_reintentable() {
        let error = ErrorDp::ColumnaFaltante("email".to_string());
        let json = serde_json::to_string(&Envoltura::fallo(&error)).unwrap();

        assert!(json.contains("\"ok\":false"));
        assert!(json.contains("\"codigo\":\"E_COLUMNA_FALTANTE\""));
        assert!(json.contains("\"reintentable\":false"));
        assert!(json.contains("\"culpa_del_cliente\":true"));
        assert!(!json.contains("resumen"));
    }

    #[test]
    fn un_fallo_interno_se_marca_reintentable() {
        let error = ErrorDp::Escritura(std::io::Error::other("disco lleno"));
        let envoltura = Envoltura::fallo(&error);
        let detalle = envoltura.error.expect("deberia traer detalle de error");

        assert_eq!(detalle.codigo, "E_ESCRITURA");
        assert!(detalle.reintentable);
        assert!(!detalle.culpa_del_cliente);
    }

    #[test]
    fn los_codigos_de_salida_distinguen_los_dos_casos() {
        assert_eq!(
            ErrorDp::ColumnaFaltante("email".to_string()).codigo_salida(),
            2
        );
        assert_eq!(
            ErrorDp::Escritura(std::io::Error::other("x")).codigo_salida(),
            1
        );
    }

    #[test]
    fn la_cadena_de_causas_llega_al_error_del_sistema() {
        let error = ErrorDp::NoPudeAbrir {
            ruta: "data/fantasma.csv".to_string(),
            origen: std::io::Error::new(std::io::ErrorKind::NotFound, "no existe"),
        };
        let causas = error.cadena_de_causas();
        assert_eq!(causas.len(), 1);
        assert!(causas[0].contains("no existe"));
    }
}
