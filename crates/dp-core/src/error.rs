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

    /// Cortafuegos de memoria: la deduplicación exacta necesita recordar cada
    /// clave única, así que sin un tope un archivo grande tumba el worker.
    #[error(
        "el archivo supera el limite de {limite} valores unicos en la clave de deduplicacion (iba en la fila {fila})"
    )]
    LimiteDeClaves { limite: usize, fila: u64 },
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
            ErrorDp::LimiteDeClaves { .. } => "E_LIMITE_DE_CLAVES",
        }
    }

    /// Si el fallo es por datos del cliente (4xx, no reintentar) o interno
    /// (5xx, reintentable).
    pub fn es_culpa_del_cliente(&self) -> bool {
        match self {
            ErrorDp::NoPudeAbrir { .. }
            | ErrorDp::CsvInvalido(_)
            | ErrorDp::OperacionDesconocida(_)
            | ErrorDp::ColumnaFaltante(_)
            // Excede el límite del plan: reintentar daría el mismo resultado.
            | ErrorDp::LimiteDeClaves { .. } => true,
            ErrorDp::Escritura(_) | ErrorDp::JsonInvalido(_) => false,
        }
    }

    /// Un error del cliente no se debe reintentar: el resultado sería el mismo.
    /// Uno interno sí, porque puede ser un fallo transitorio de disco o red.
    pub fn es_reintentable(&self) -> bool {
        !self.es_culpa_del_cliente()
    }

    /// Código de salida del proceso, que es lo ÚNICO que un orquestador de
    /// contenedores puede leer sin interpretar texto.
    ///
    /// Convención del motor:
    ///   0 = todo bien
    ///   1 = fallo interno, reintentar el job
    ///   2 = datos del cliente inválidos, marcar FAILED y no reintentar
    pub fn codigo_salida(&self) -> i32 {
        if self.es_culpa_del_cliente() { 2 } else { 1 }
    }

    /// La cadena de causas aplanada a texto, de la más externa a la más interna.
    pub fn cadena_de_causas(&self) -> Vec<String> {
        let mut causas = Vec::new();
        let mut actual = std::error::Error::source(self);
        while let Some(error) = actual {
            causas.push(error.to_string());
            actual = error.source();
        }
        causas
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
