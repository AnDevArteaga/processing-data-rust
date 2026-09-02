use super::{Operacion, abrir_csv, verificar_columnas};
use crate::error::Resultado;
use crate::modelo::{ClienteBruto, Resumen};
use crate::normalizacion::{normalizar_email, procesar_fila};
use crate::sumidero::Sumidero;
use std::collections::HashSet;

/// Una unit struct: sin campos, existe solo para colgarle la implementación
/// del trait. No ocupa memoria.
pub struct LimpiarCsv;

impl Operacion for LimpiarCsv {
    fn nombre(&self) -> &'static str {
        "csv.clean"
    }

    fn descripcion(&self) -> &'static str {
        "normaliza, valida y deduplica por email"
    }

    fn ejecutar(&self, entrada: &str, sumidero: &mut dyn Sumidero) -> Resultado<Resumen> {
        let mut lector = abrir_csv(entrada)?;
        verificar_columnas(&mut lector)?;

        // OJO: la memoria de esta operación crece con el número de claves
        // únicas, no con el tamaño del archivo. Con 2,5 millones de emails
        // distintos son unos 240 MB. Hay que limitarlo por plan.
        let mut vistos: HashSet<String> = HashSet::new();
        let mut r = Resumen::default();

        for resultado in lector.deserialize::<ClienteBruto>() {
            let bruto: ClienteBruto = resultado?;
            r.leidas += 1;

            let email = normalizar_email(&bruto.email);
            if !vistos.insert(email) {
                r.duplicadas += 1;
                continue;
            }

            let limpio = procesar_fila(&bruto, &mut r);
            sumidero.escribir(&limpio)?;
            r.escritas += 1;
        }

        sumidero.cerrar()?;
        Ok(r)
    }
}
