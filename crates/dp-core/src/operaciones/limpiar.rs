use super::{Operacion, recorrer};
use crate::error::Resultado;
use crate::limpieza::Config;
use crate::modelo::Resumen;
use crate::sumidero::Sumidero;

/// Ya no es una unit struct: ahora lleva la configuración del job.
///
/// `#[derive(Default)]` funciona porque `Config` también implementa `Default`:
/// el derive simplemente llama al `default()` de cada campo.
#[derive(Default)]
pub struct LimpiarCsv {
    pub config: Config,
}

impl Operacion for LimpiarCsv {
    fn nombre(&self) -> &'static str {
        "csv.clean"
    }

    fn descripcion(&self) -> &'static str {
        "normaliza, valida y deduplica por las columnas que elijas"
    }

    fn ejecutar(&self, entrada: &str, sumidero: &mut dyn Sumidero) -> Resultado<Resumen> {
        recorrer(entrada, &self.config, sumidero, true)
    }
}
