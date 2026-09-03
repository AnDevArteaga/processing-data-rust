//! Catálogo de operaciones de procesamiento.
//!
//! `//!` es un comentario de documentación que describe el módulo que lo
//! contiene, a diferencia de `///` que documenta el elemento siguiente.

use crate::error::{ErrorDp, Resultado};
use crate::fila::Encabezados;
use crate::limpieza::{Config, Limpiador};
use crate::modelo::Resumen;
use crate::sumidero::Sumidero;
use rustc_hash::FxHashSet;
use std::fs::File;

// `mod` declara los submódulos, que Cargo busca en limpiar.rs e inspeccionar.rs.
// Son privados: solo este módulo los ve.
mod inspeccionar;
mod limpiar;

// `pub use` los re-exporta hacia afuera. Quien use la librería escribe
// `dp_core::operaciones::LimpiarCsv` sin enterarse de en qué archivo vive.
pub use inspeccionar::InspeccionarCsv;
pub use limpiar::LimpiarCsv;

/// La interfaz de operación que pide el PDF: cada procesador (CSV hoy, PDF y
/// OCR mañana) implementa este trait y el motor ignora los detalles.
/// `Send` porque el worker mueve la operación a `spawn_blocking`.
pub trait Operacion: Send {
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

/// El bucle que comparten `csv.clean` y `csv.inspect`.
///
/// Antes estaba copiado en los dos archivos, con el riesgo obvio de que un
/// arreglo se aplicara solo en uno. `escribir` es la única diferencia real
/// entre las dos operaciones.
fn recorrer(
    entrada: &str,
    config: &Config,
    sumidero: &mut dyn Sumidero,
    escribir: bool,
) -> Resultado<Resumen> {
    let mut lector = abrir_csv(entrada)?;

    // `headers()` toma prestado el lector como mutable, pero `desde_csv` copia
    // los nombres a memoria propia, así que el préstamo muere en esta línea y
    // el bucle de abajo puede volver a usar el lector.
    let encabezados = Encabezados::desde_csv(lector.headers()?);

    // Validar el esquema ANTES de procesar: si falta una columna, el job muere
    // aquí y no a mitad del archivo con medio resultado ya escrito.
    let limpiador = Limpiador::nuevo(&encabezados, config)?;

    if escribir {
        sumidero.encabezado(limpiador.columnas_salida())?;
    }

    let mut vistos: FxHashSet<Box<str>> = FxHashSet::default();
    let mut r = Resumen::default();

    // Los tres buffers se asignan UNA vez y se reutilizan en cada fila. Con
    // millones de filas, la diferencia entre esto y crear un String nuevo por
    // fila es medible.
    let mut registro = csv::StringRecord::new();
    let mut clave = String::new();
    let mut salida: Vec<String> = Vec::new();

    // `read_record` devuelve false al llegar al final y escribe la fila dentro
    // del buffer que le pasamos, en vez de devolver uno nuevo.
    while lector.read_record(&mut registro)? {
        r.leidas += 1;

        if limpiador.deduplica() {
            limpiador.construir_clave(&registro, &mut clave);

            // Preguntar antes de insertar tiene una ventaja concreta: una clave
            // repetida no reserva memoria nueva. Solo las claves nuevas pagan
            // la asignación del Box<str>.
            if vistos.contains(clave.as_str()) {
                r.duplicadas += 1;
                continue;
            }

            // El cortafuegos va aquí, después de descartar duplicados: un
            // archivo de mil millones de filas con pocas claves distintas no
            // debe tropezar con el límite.
            if vistos.len() >= config.limite_claves {
                return Err(ErrorDp::LimiteDeClaves {
                    limite: config.limite_claves,
                    fila: r.leidas,
                });
            }

            // `Box<str>` implementa `Borrow<str>`, y por eso el `contains` de
            // arriba acepta un `&str` sin construir el Box.
            vistos.insert(clave.as_str().into());
        }

        limpiador.limpiar(&registro, &mut r, &mut salida);

        if escribir {
            sumidero.escribir(&salida)?;
            r.escritas += 1;
        }
    }

    sumidero.cerrar()?;
    Ok(r)
}

/// Las operaciones disponibles con la configuración por defecto.
///
/// `Box<dyn Operacion>` es despacho dinámico: el tipo concreto se resuelve en
/// ejecución, que es obligatorio aquí porque el nombre de la operación llega
/// como texto desde la API o desde la cola.
pub fn catalogo() -> Vec<Box<dyn Operacion>> {
    catalogo_con(Config::default())
}

pub fn catalogo_con(config: Config) -> Vec<Box<dyn Operacion>> {
    vec![
        Box::new(LimpiarCsv {
            config: config.clone(),
        }),
        Box::new(InspeccionarCsv { config }),
    ]
}

pub fn buscar_operacion(nombre: &str, config: Config) -> Option<Box<dyn Operacion>> {
    catalogo_con(config)
        .into_iter()
        .find(|op| op.nombre() == nombre)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sumidero::{SumideroMemoria, SumideroNulo};

    /// Los tests se ejecutan con el directorio de trabajo en la raíz del
    /// PAQUETE (crates/dp-core), no del workspace. Por eso subimos dos niveles
    /// para llegar a `data/`, que vive en la raíz del repositorio.
    ///
    /// `env!` lee una variable de entorno EN TIEMPO DE COMPILACIÓN, y Cargo
    /// define `CARGO_MANIFEST_DIR` con la ruta del Cargo.toml de este paquete.
    fn ruta_datos(nombre: &str) -> String {
        format!("{}/../../data/{}", env!("CARGO_MANIFEST_DIR"), nombre)
    }

    fn limpiar(config: Config) -> LimpiarCsv {
        LimpiarCsv { config }
    }

    #[test]
    fn el_catalogo_expone_las_operaciones_por_nombre() {
        let cfg = Config::default();
        assert!(buscar_operacion("csv.clean", cfg.clone()).is_some());
        assert!(buscar_operacion("csv.inspect", cfg.clone()).is_some());
        assert!(buscar_operacion("csv.noexiste", cfg).is_none());
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
        let mut sumidero = SumideroMemoria::default();
        let error = limpiar(Config::default())
            .ejecutar(&ruta_datos("sin_columna.csv"), &mut sumidero)
            .expect_err("deberia rechazar un CSV sin la columna email");

        assert_eq!(error.codigo(), "E_COLUMNA_FALTANTE");
        // Y no escribió ni el encabezado: falló antes de tocar la salida.
        assert!(sumidero.columnas.is_empty());
    }

    /// Test de integración: la operación completa sobre el CSV real.
    #[test]
    fn limpiar_csv_produce_el_resumen_esperado() {
        let mut sumidero = SumideroNulo;
        let resumen = limpiar(Config::default())
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
        let mut sumidero = SumideroMemoria::default();
        let resumen = InspeccionarCsv {
            config: Config::default(),
        }
        .ejecutar(&ruta_datos("clientes.csv"), &mut sumidero)
        .expect("deberia inspeccionar sin errores");

        assert_eq!(resumen.leidas, 15);
        assert_eq!(resumen.duplicadas, 3);
        assert_eq!(resumen.escritas, 0);
        assert!(sumidero.filas.is_empty());
        assert!(
            !InspeccionarCsv {
                config: Config::default()
            }
            .produce_archivo()
        );
    }

    /// El cortafuegos de memoria: con tope 2, el CSV de prueba (que tiene 12
    /// emails únicos) debe abortar en vez de seguir creciendo.
    #[test]
    fn superar_el_limite_de_claves_aborta_el_job() {
        let mut sumidero = SumideroNulo;
        let config = Config {
            limite_claves: 2,
            ..Config::default()
        };
        let error = limpiar(config)
            .ejecutar(&ruta_datos("clientes.csv"), &mut sumidero)
            .expect_err("deberia abortar al pasar el tope de claves unicas");

        assert_eq!(error.codigo(), "E_LIMITE_DE_CLAVES");
        // Es un límite de plan: reintentar daría exactamente el mismo error.
        assert!(!error.es_reintentable());
        assert_eq!(error.codigo_salida(), 2);
    }

    /// Y con un tope holgado el resultado no cambia: el cortafuegos no debe
    /// alterar el comportamiento normal.
    #[test]
    fn un_limite_holgado_no_cambia_el_resultado() {
        let mut sumidero = SumideroNulo;
        let config = Config {
            limite_claves: 1000,
            ..Config::default()
        };
        let resumen = limpiar(config)
            .ejecutar(&ruta_datos("clientes.csv"), &mut sumidero)
            .expect("deberia procesar sin tocar el limite");

        assert_eq!(resumen.escritas, 12);
        assert_eq!(resumen.duplicadas, 3);
    }

    /// El hallazgo 2 en su forma completa: deduplicar por email deja pasar dos
    /// filas que comparten teléfono, y deduplicar por las dos columnas a la vez
    /// las distingue igual, pero deduplicar SOLO por teléfono las colapsa.
    #[test]
    fn la_clave_de_deduplicacion_cambia_el_resultado() {
        let por_email = Config::default();
        let por_telefono = Config {
            claves: vec!["telefono".to_string()],
            ..Config::default()
        };
        let por_ambas = Config {
            claves: vec!["email".to_string(), "telefono".to_string()],
            ..Config::default()
        };

        let mut resultados = Vec::new();
        for config in [por_email, por_telefono, por_ambas] {
            let mut sumidero = SumideroNulo;
            let resumen = limpiar(config)
                .ejecutar(&ruta_datos("clientes.csv"), &mut sumidero)
                .expect("deberia procesar");
            resultados.push(resumen.duplicadas);
        }

        // Por email: 3 duplicados. Por teléfono se colapsan más filas, porque
        // varios registros comparten número (incluidos los vacíos, que
        // normalizan a la clave vacía).
        assert_eq!(resultados[0], 3);
        assert!(
            resultados[1] > resultados[0],
            "por telefono deberia colapsar mas filas que por email, fue {:?}",
            resultados
        );
        // Por las dos: la clave es más específica, así que hay menos duplicados.
        assert!(resultados[2] <= resultados[0]);
    }

    /// Sin clave no se deduplica: salen todas las filas leídas.
    #[test]
    fn sin_clave_no_se_pierde_ninguna_fila() {
        let mut sumidero = SumideroMemoria::default();
        let config = Config {
            claves: vec![],
            ..Config::default()
        };
        let resumen = limpiar(config)
            .ejecutar(&ruta_datos("clientes.csv"), &mut sumidero)
            .expect("deberia procesar");

        assert_eq!(resumen.leidas, 15);
        assert_eq!(resumen.escritas, 15);
        assert_eq!(resumen.duplicadas, 0);
        assert_eq!(sumidero.filas.len(), 15);
    }

    /// El sumidero ya no sabe de clientes: verificamos que el encabezado que
    /// recibe sale de la configuración, no de una struct fija.
    #[test]
    fn el_sumidero_recibe_el_encabezado_de_las_columnas_configuradas() {
        let mut sumidero = SumideroMemoria::default();
        let config = Config {
            emails: vec![],
            telefonos: vec![],
            claves: vec!["nombre".to_string()],
            ..Config::default()
        };
        limpiar(config)
            .ejecutar(&ruta_datos("clientes.csv"), &mut sumidero)
            .expect("deberia procesar");

        // Sin columnas de email ni telefono configuradas, no hay columnas de
        // diagnostico: el encabezado es exactamente el del archivo de entrada.
        assert_eq!(sumidero.columnas, ["nombre", "email", "telefono", "ciudad"]);
        assert_eq!(sumidero.filas[0].len(), 4);
    }
}
