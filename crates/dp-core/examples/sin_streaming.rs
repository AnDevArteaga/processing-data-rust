// La forma INGENUA: cargar el archivo completo en memoria antes de procesarlo.
// Es lo que hace por defecto casi cualquier script en Python o Node.
// Córrelo con:  cargo run --release --example sin_streaming -- data/clientes_grande.csv

use std::env;
use std::error::Error;
use std::fs;

fn main() -> Result<(), Box<dyn Error>> {
    let ruta = env::args()
        .nth(1)
        .unwrap_or_else(|| "data/clientes.csv".to_string());

    // Esta única línea reserva en el heap tantos bytes como pese el archivo.
    let contenido: String = fs::read_to_string(&ruta)?;

    let mut filas: u64 = 0;
    for _linea in contenido.lines().skip(1) {
        filas += 1;
    }

    println!("Filas: {filas}");
    println!("String en memoria: {} MB", contenido.len() / 1_048_576);

    Ok(())
}
