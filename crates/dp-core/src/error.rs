use thiserror::Error;

/// Todos los errores que este motor puede producir, enumerados.
///
/// `pub` hace el tipo visible fuera de este módulo. Sin `pub`, un módulo
/// hermano no podría ni nombrarlo: en Rust todo es privado por defecto.
#[derive(Debug, Error)]
pub enum ErrorDp {
    #[error("no pude abrir el archivo '{ruta}'")]
    NoPudeAbrir {
        ruta: String,
        #[source]
        origen: std::io::Error,
    },

    #[error("el CSV está mal formado: {0}")]
    CsvInvalido(#[from] csv::Error),

    #[error("no pude escribir la salida: {0}")]
    Escritura(#[from] std::io::Error),

    #[error("no pude generar el JSON: {0}")]
    JsonInvalido(#[from] serde_json::Error),

    #[error("operación desconocida: '{0}'")]
    OperacionDesconocida(String),

    #[error("al archivo le falta la columna obligatoria '{0}'")]
    ColumnaFaltante(String),
}

impl ErrorDp {
    /// Código estable para agrupar y alertar (sección 12 del PDF).
    ///
    /// El `match` no tiene rama por defecto: agregar una variante al enum
    /// rompe la compilación aquí hasta que le asignes su código.
    pub fn codigo(&self) -> &'static str {
        match self {
            ErrorDp::NoPudeAbrir { .. } => "E_ARCHIVO_NO_ENCONTRADO",
            ErrorDp::CsvInvalido(_) => "E_CSV_INVALIDO",
            ErrorDp::Escritura(_) => "E_ESCRITURA",
            ErrorDp::JsonInvalido(_) => "E_JSON",
            ErrorDp::OperacionDesconocida(_) => "E_OPERACION_DESCONOCIDA",
            ErrorDp::ColumnaFaltante(_) => "E_COLUMNA_FALTANTE",
        }
    }

    /// Si el fallo es por datos del cliente (4xx, no reintentar) o interno
    /// (5xx, reintentable).
    pub fn es_culpa_del_cliente(&self) -> bool {
        match self {
            ErrorDp::NoPudeAbrir { .. }
            | ErrorDp::CsvInvalido(_)
            | ErrorDp::OperacionDesconocida(_)
            | ErrorDp::ColumnaFaltante(_) => true,
            ErrorDp::Escritura(_) | ErrorDp::JsonInvalido(_) => false,
        }
    }
}

/// Alias de tipo para no repetir el error en cada firma.
pub type Resultado<T> = Result<T, ErrorDp>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cada_variante_de_error_tiene_su_codigo() {
        let e = ErrorDp::OperacionDesconocida("pdf.ocr".to_string());
        assert_eq!(e.codigo(), "E_OPERACION_DESCONOCIDA");
        assert!(e.es_culpa_del_cliente());

        let e = ErrorDp::ColumnaFaltante("email".to_string());
        assert_eq!(e.codigo(), "E_COLUMNA_FALTANTE");
        assert!(e.es_culpa_del_cliente());
    }

    #[test]
    fn los_fallos_internos_son_reintentables() {
        let e = ErrorDp::Escritura(std::io::Error::other("disco lleno"));
        assert_eq!(e.codigo(), "E_ESCRITURA");
        assert!(!e.es_culpa_del_cliente());
    }
}
