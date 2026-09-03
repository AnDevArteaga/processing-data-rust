//! Adónde van las filas procesadas.
//!
//! El trait ya NO habla de clientes. Antes su firma era
//! `escribir(&mut self, cliente: &ClienteLimpio)`, lo que obligaba a que toda
//! operación futura produjera clientes: una operación de OCR sobre facturas no
//! tenía dónde encajar. Ahora el contrato es "un encabezado y filas de
//! valores", que es lo que cualquier operación tabular puede producir.
//!
//! El precio de esta generalidad es que perdemos el chequeo de campos en tiempo
//! de compilación: todo es `String`. Es un intercambio deliberado, porque el
//! esquema lo decide el CSV que sube el cliente y no lo podemos conocer al
//! compilar. La validación se mueve a `Limpiador::nuevo`, que falla temprano
//! con el nombre de la columna que falta.

use crate::error::{ErrorDp, Resultado};
use std::fs::File;
use std::io::{BufWriter, Write};

/// `Send` porque el worker ejecuta el motor en `spawn_blocking`: el sumidero
/// cruza de un hilo del runtime a un hilo de la reserva bloqueante.
pub trait Sumidero: Send {
    /// Se llama una sola vez, antes de la primera fila.
    fn encabezado(&mut self, columnas: &[String]) -> Resultado<()>;

    /// Una fila. `valores` viene en el mismo orden que `columnas`.
    fn escribir(&mut self, valores: &[String]) -> Resultado<()>;

    /// Vaciar buffers y cerrar. Sin esto se pierden las últimas filas.
    fn cerrar(&mut self) -> Resultado<()>;
}

pub struct SumideroCsv {
    escritor: csv::Writer<File>,
}

impl SumideroCsv {
    pub fn nuevo(ruta: &str) -> Resultado<Self> {
        // Abrimos el archivo a mano en vez de usar `Writer::from_path` para
        // poder incluir la ruta en el mensaje de error.
        let archivo = File::create(ruta).map_err(|origen| ErrorDp::NoPudeAbrir {
            ruta: ruta.to_string(),
            origen,
        })?;
        Ok(SumideroCsv {
            escritor: csv::Writer::from_writer(archivo),
        })
    }
}

impl Sumidero for SumideroCsv {
    fn encabezado(&mut self, columnas: &[String]) -> Resultado<()> {
        self.escritor.write_record(columnas)?;
        Ok(())
    }

    fn escribir(&mut self, valores: &[String]) -> Resultado<()> {
        self.escritor.write_record(valores)?;
        Ok(())
    }

    fn cerrar(&mut self) -> Resultado<()> {
        self.escritor.flush()?;
        Ok(())
    }
}

pub struct SumideroJson {
    salida: BufWriter<File>,
    columnas: Vec<String>,
    primero: bool,
}

impl SumideroJson {
    pub fn nuevo(ruta: &str) -> Resultado<Self> {
        let archivo = File::create(ruta).map_err(|origen| ErrorDp::NoPudeAbrir {
            ruta: ruta.to_string(),
            origen,
        })?;
        let mut salida = BufWriter::new(archivo);
        writeln!(salida, "[")?;
        Ok(SumideroJson {
            salida,
            columnas: Vec::new(),
            primero: true,
        })
    }
}

impl Sumidero for SumideroJson {
    fn encabezado(&mut self, columnas: &[String]) -> Resultado<()> {
        self.columnas = columnas.to_vec();
        Ok(())
    }

    fn escribir(&mut self, valores: &[String]) -> Resultado<()> {
        // Construimos el objeto a mano en vez de usar `serde_json::Map` porque
        // Map es un BTreeMap: ordenaría las claves alfabéticamente y perderíamos
        // el orden de columnas del archivo. `to_string` sobre cada texto se
        // encarga del escapado (comillas, saltos de línea, acentos).
        if self.primero {
            write!(self.salida, "  {{")?;
            self.primero = false;
        } else {
            write!(self.salida, ",\n  {{")?;
        }

        for (posicion, valor) in valores.iter().enumerate() {
            if posicion > 0 {
                write!(self.salida, ",")?;
            }
            let nombre = self
                .columnas
                .get(posicion)
                .map(String::as_str)
                .unwrap_or("desconocida");
            write!(
                self.salida,
                "{}:{}",
                serde_json::to_string(nombre)?,
                serde_json::to_string(valor)?
            )?;
        }

        write!(self.salida, "}}")?;
        Ok(())
    }

    fn cerrar(&mut self) -> Resultado<()> {
        // Si no se escribió ninguna fila no hay que meter el salto de línea,
        // o el JSON quedaría como "[\n\n]".
        if !self.primero {
            writeln!(self.salida)?;
        }
        writeln!(self.salida, "]")?;
        self.salida.flush()?;
        Ok(())
    }
}

/// Descarta todo. Para operaciones que solo inspeccionan.
pub struct SumideroNulo;

impl Sumidero for SumideroNulo {
    fn encabezado(&mut self, _columnas: &[String]) -> Resultado<()> {
        Ok(())
    }

    fn escribir(&mut self, _valores: &[String]) -> Resultado<()> {
        Ok(())
    }

    fn cerrar(&mut self) -> Resultado<()> {
        Ok(())
    }
}

/// Guarda las filas en memoria. Solo para tests: permite verificar lo que una
/// operación produce sin tocar el disco.
#[derive(Default)]
pub struct SumideroMemoria {
    pub columnas: Vec<String>,
    pub filas: Vec<Vec<String>>,
}

impl Sumidero for SumideroMemoria {
    fn encabezado(&mut self, columnas: &[String]) -> Resultado<()> {
        self.columnas = columnas.to_vec();
        Ok(())
    }

    fn escribir(&mut self, valores: &[String]) -> Resultado<()> {
        self.filas.push(valores.to_vec());
        Ok(())
    }

    fn cerrar(&mut self) -> Resultado<()> {
        Ok(())
    }
}
