//! Catálogo de operaciones de procesamiento.
//!
//! `//!` es un comentario de documentación que describe el módulo que lo
//! contiene, a diferencia de `///` que documenta el elemento siguiente.

use crate::error::{ErrorDp, Resultado};
use crate::modelo::Resumen;
use crate::sumidero::Sumidero;
use std::fs::File;

// `mod` declara los submódulos, que Cargo busca en limpiar.rs e inspeccionar.rs.
// Son privados: solo este módulo los ve.
mod inspeccionar;
mod limpiar;

// `pub use` los re-exporta hacia afuera. Quien use la librería escribe
// `dp::operaciones::LimpiarCsv` sin enterarse de en qué archivo vive.
pub use inspeccionar::InspeccionarCsv;
pub use limpiar::LimpiarCsv;

/// La interfaz de operación que pide el PDF: cada procesador (CSV hoy, PDF y
/// OCR mañana) implementa este trait y el motor ignora los detalles.
pub trait Operacion {
    /// El identificador que viaja en el payload de la API:
    /// `POST /v1/jobs {"operation": "csv.clean"}`
    fn nombre(&self) -> &'static str;

    fn descripcion(&self) -> &'static str;

    /// Método con cuerpo por defecto: quien implemente el trait puede
    /// sobrescribirlo o heredar esta versión.
    fn produce_archivo(&self) -> bool {
        true
    }

    fn ejecutar(&self, entrada: &str, sumidero: &mut dyn Sumidero) -> Resultado<Resumen>;
}

/// Abre un CSV incluyendo la ruta en el mensaje de error.
/// `map_err` transforma el error de un Result sin tocar el caso de éxito.
pub fn abrir_csv(ruta: &str) -> Resultado<csv::Reader<File>> {
    let archivo = File::open(ruta).map_err(|origen| ErrorDp::NoPudeAbrir {
        ruta: ruta.to_string(),
        origen,
    })?;
    Ok(csv::Reader::from_reader(archivo))
}

/// Falla temprano si al archivo le falta alguna columna obligatoria, antes de
/// gastar CPU procesando filas.
pub fn verificar_columnas(lector: &mut csv::Reader<File>) -> Resultado<()> {
    let encabezados = lector.headers()?.clone();
    for obligatoria in ["nombre", "email", "telefono", "ciudad"] {
        if !encabezados.iter().any(|c| c.trim() == obligatoria) {
            return Err(ErrorDp::ColumnaFaltante(obligatoria.to_string()));
        }
    }
    Ok(())
}

/// Las operaciones disponibles.
///
/// `Box<dyn Operacion>` es despacho dinámico: el tipo concreto se resuelve en
/// ejecución, que es obligatorio aquí porque el nombre de la operación llega
/// como texto desde la API o desde la cola.
pub fn catalogo() -> Vec<Box<dyn Operacion>> {
    vec![Box::new(LimpiarCsv), Box::new(InspeccionarCsv)]
}

pub fn buscar_operacion(nombre: &str) -> Option<Box<dyn Operacion>> {
    catalogo().into_iter().find(|op| op.nombre() == nombre)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sumidero::SumideroNulo;

    /// Los tests se ejecutan con el directorio de trabajo en la raíz del
    /// PAQUETE (crates/dp-core), no del workspace. Por eso subimos dos niveles
    /// para llegar a `data/`, que vive en la raíz del repositorio.
    ///
    /// `env!` lee una variable de entorno EN TIEMPO DE COMPILACIÓN, y Cargo
    /// define `CARGO_MANIFEST_DIR` con la ruta del Cargo.toml de este paquete.
    fn ruta_datos(nombre: &str) -> String {
        format!("{}/../../data/{}", env!("CARGO_MANIFEST_DIR"), nombre)
    }

    #[test]
    fn el_catalogo_expone_las_operaciones_por_nombre() {
        assert!(buscar_operacion("csv.clean").is_some());
        assert!(buscar_operacion("csv.inspect").is_some());
        assert!(buscar_operacion("csv.noexiste").is_none());
    }

    #[test]
    fn archivo_inexistente_da_error_de_archivo() {
        let error = abrir_csv(&ruta_datos("no_existe_este_archivo.csv"))
            .expect_err("deberia fallar con un archivo que no existe");
        assert_eq!(error.codigo(), "E_ARCHIVO_NO_ENCONTRADO");
        assert!(error.to_string().contains("no_existe_este_archivo.csv"));
    }

    #[test]
    fn csv_sin_columna_obligatoria_falla_antes_de_procesar() {
        let mut sumidero = SumideroNulo;
        let error = LimpiarCsv
            .ejecutar(&ruta_datos("sin_columna.csv"), &mut sumidero)
            .expect_err("deberia rechazar un CSV sin la columna email");
        assert_eq!(error.codigo(), "E_COLUMNA_FALTANTE");
    }

    /// Test de integración: la operación completa sobre el CSV real.
    #[test]
    fn limpiar_csv_produce_el_resumen_esperado() {
        let mut sumidero = SumideroNulo;
        let resumen = LimpiarCsv
            .ejecutar(&ruta_datos("clientes.csv"), &mut sumidero)
            .expect("el CSV de prueba deberia procesarse sin errores");

        assert_eq!(
            resumen,
            Resumen {
                leidas: 15,
                escritas: 12,
                duplicadas: 3,
                emails_invalidos: 2,
                telefonos_invalidos: 1,
                telefonos_vacios: 1,
                campos_normalizados: 5,
            }
        );
    }

    /// `csv.inspect` cuenta lo mismo que `csv.clean` pero sin escribir nada.
    #[test]
    fn inspeccionar_cuenta_igual_pero_no_escribe() {
        let mut sumidero = SumideroNulo;
        let resumen = InspeccionarCsv
            .ejecutar(&ruta_datos("clientes.csv"), &mut sumidero)
            .expect("deberia inspeccionar sin errores");

        assert_eq!(resumen.leidas, 15);
        assert_eq!(resumen.duplicadas, 3);
        assert_eq!(resumen.escritas, 0);
        assert!(!InspeccionarCsv.produce_archivo());
    }
}
