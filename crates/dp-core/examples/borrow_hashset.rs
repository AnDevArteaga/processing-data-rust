// ¿Por qué el HashSet tiene que ser DUEÑO de sus claves?
// Córrelo con:  cargo run --example borrow_hashset

use std::collections::HashSet;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let mut lector = csv::Reader::from_path("data/clientes.csv")?;

    // ------------------------------------------------------------------
    // LO QUE NO COMPILA
    //
    // let mut vistos: HashSet<&str> = HashSet::new();
    //
    // for resultado in lector.records() {
    //     let fila = resultado?;
    //     let clave: &str = &fila[1];
    //     vistos.insert(clave);
    // }
    //
    // El compilador responde:
    //
    //   error[E0597]: `fila` does not live long enough
    //      |
    //   15 |     let fila = resultado?;
    //      |         ---- binding `fila` declared here
    //   18 |     let clave: &str = &fila[1];
    //      |                        ^^^^ borrowed value does not live long enough
    //   25 |     }
    //      |     - `fila` dropped here while still borrowed
    //   27 |     println!("vistos: {}", vistos.len());
    //      |                            ------ borrow later used here
    //
    // Motivo: `fila` muere al final de cada iteración, y la librería csv
    // reutiliza su buffer interno para la fila siguiente. La referencia
    // guardada en el set apuntaría a memoria ya reescrita. En C o C++ esto
    // compilaría y tendrías un use-after-free silencioso.
    // ------------------------------------------------------------------

    // LO QUE SÍ COMPILA: el set es dueño de sus Strings.
    let mut vistos: HashSet<String> = HashSet::new();

    for resultado in lector.records() {
        let fila = resultado?;
        // `to_lowercase` produce un String nuevo del que el set se vuelve dueño.
        // Esta asignación de memoria no es una muleta: es un requisito.
        vistos.insert(fila[1].trim().to_lowercase());
    }

    println!("emails unicos: {}", vistos.len());
    Ok(())
}
