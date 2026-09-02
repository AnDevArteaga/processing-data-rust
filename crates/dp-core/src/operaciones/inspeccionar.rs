use super::{Operacion, abrir_csv, verificar_columnas};
use crate::error::Resultado;
use crate::modelo::{ClienteBruto, Resumen};
use crate::normalizacion::{normalizar_email, procesar_fila};
use crate::sumidero::Sumidero;
use std::collections::HashSet;

/// Operación barata: diagnostica sin escribir salida. Sirve como vista previa
/// antes de cobrarle al cliente el job completo.
pub struct InspeccionarCsv;

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

    fn ejecutar(&self, entrada: &str, _sumidero: &mut dyn Sumidero) -> Resultado<Resumen> {
        let mut lector = abrir_csv(entrada)?;
        verificar_columnas(&mut lector)?;

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

            // Descartamos la fila limpia: solo queremos los contadores.
            let _ = procesar_fila(&bruto, &mut r);
        }

        Ok(r)
    }
}
