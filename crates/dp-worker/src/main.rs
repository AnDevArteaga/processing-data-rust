//! Binario autónomo del worker.
//!
//! En esta fase la cola vive en memoria dentro del proceso de la API, así
//! que este binario no vería los jobs. El worker corre embebido en `dp-api`.
//! Cuando la cola sea PostgreSQL, este proceso se arranca por separado y
//! comparte el mismo repositorio.

fn main() {
    eprintln!("dp-worker: en esta fase la cola vive en memoria.");
    eprintln!("Arranca `cargo run -p dp-api`: el worker corre dentro de ese proceso.");
    eprintln!("Este binario queda para cuando el repositorio sea PostgreSQL.");
    std::process::exit(2);
}
