// Demostración de ownership, borrowing y move.
// Córrelo con:  cargo run --example ownership

use std::fs;

// OPCIÓN CORRECTA: pedimos un préstamo (&str) en vez de la propiedad.
// La función puede leer el texto pero no es su dueña, así que al terminar
// no libera nada y quien llamó sigue siendo dueño de su String.
fn contar_lineas(texto: &str) -> u64 {
    texto.lines().count() as u64
}

// OPCIÓN QUE MUEVE: esta versión se vuelve dueña y destruye el texto al terminar.
// Solo tiene sentido cuando de verdad quieres consumir el valor.
fn consumir_texto(texto: String) -> usize {
    texto.len()
}

fn main() {
    let contenido: String = fs::read_to_string("data/clientes.csv").expect("no pude leer");

    // Prestamos el String tantas veces como queramos: no se mueve nada.
    println!("lineas: {}", contar_lineas(&contenido));
    println!("lineas otra vez: {}", contar_lineas(&contenido));
    println!("bytes: {}", contenido.len());

    // Esta llamada sí mueve el valor, y tiene que ir de última:
    // después de esta línea `contenido` ya no existe.
    let bytes = consumir_texto(contenido);
    println!("bytes tras consumir: {bytes}");

    // Si descomentas la línea siguiente, el programa deja de compilar
    // con el mismo error E0382 de antes:
    // println!("{}", contenido.len());
}
