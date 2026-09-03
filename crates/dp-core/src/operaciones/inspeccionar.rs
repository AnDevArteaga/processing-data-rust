use super::{Operacion, recorrer};
use crate::error::Resultado;
use crate::limpieza::Config;
use crate::modelo::Resumen;
use crate::sumidero::Sumidero;

/// Operación barata: diagnostica sin escribir salida. Sirve como vista previa
/// antes de cobrarle al cliente el job completo.
#[derive(Default)]
pub struct InspeccionarCsv {
    pub config: Config,
}

impl Operacion for InspeccionarCsv {
    fn nombre(&self) -> &'static str {
        "csv.inspect"
    }

    fn descripcion(&self) -> &'static str {
        "cuenta filas y problemas sin generar salida"
    }

    fn produce_archivo(&self) -> bool {
        false
    }

    fn ejecutar(&self, entrada: &str, sumidero: &mut dyn Sumidero) -> Resultado<Resumen> {
        // El `false` final es toda la diferencia con csv.clean.
        recorrer(entrada, &self.config, sumidero, false)
    }
}
