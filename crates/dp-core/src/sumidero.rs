use crate::error::{ErrorDp, Resultado};
use crate::modelo::ClienteLimpio;
use std::fs::File;
use std::io::{BufWriter, Write};

/// Adónde van las filas procesadas. La operación escribe sin saber si el
/// destino es CSV, JSON o la basura.
pub trait Sumidero {
    fn escribir(&mut self, cliente: &ClienteLimpio) -> Resultado<()>;
    fn cerrar(&mut self) -> Resultado<()>;
}

pub struct SumideroCsv {
    escritor: csv::Writer<File>,
}

impl SumideroCsv {
    pub fn nuevo(ruta: &str) -> Resultado<Self> {
        Ok(SumideroCsv {
            escritor: csv::Writer::from_path(ruta)?,
        })
    }
}

impl Sumidero for SumideroCsv {
    fn escribir(&mut self, cliente: &ClienteLimpio) -> Resultado<()> {
        self.escritor.serialize(cliente)?;
        Ok(())
    }

    fn cerrar(&mut self) -> Resultado<()> {
        self.escritor.flush()?;
        Ok(())
    }
}

pub struct SumideroJson {
    salida: BufWriter<File>,
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
            primero: true,
        })
    }
}

impl Sumidero for SumideroJson {
    fn escribir(&mut self, cliente: &ClienteLimpio) -> Resultado<()> {
        let json = serde_json::to_string(cliente)?;
        if self.primero {
            write!(self.salida, "  {json}")?;
            self.primero = false;
        } else {
            write!(self.salida, ",\n  {json}")?;
        }
        Ok(())
    }

    fn cerrar(&mut self) -> Resultado<()> {
        writeln!(self.salida)?;
        writeln!(self.salida, "]")?;
        self.salida.flush()?;
        Ok(())
    }
}

/// Descarta todo. Para operaciones que solo inspeccionan.
pub struct SumideroNulo;

impl Sumidero for SumideroNulo {
    fn escribir(&mut self, _cliente: &ClienteLimpio) -> Resultado<()> {
        Ok(())
    }

    fn cerrar(&mut self) -> Resultado<()> {
        Ok(())
    }
}
