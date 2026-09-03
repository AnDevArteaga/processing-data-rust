//! Implementaciones de los puertos que no necesitan infraestructura.
//!
//! No son un juguete: son lo que permite que la suite de tests de integración
//! de la API corra en milisegundos sin base de datos. Un test que tarda cinco
//! segundos en levantar contenedores es un test que nadie ejecuta.
//!
//! El repositorio en memoria se sustituye por `dp-postgres` en la fase 2, y el
//! almacén local por `dp-almacen-s3` en la fase 4. Ninguno de los dos cambios
//! toca la API ni el worker.

pub mod almacen_local;
pub mod reloj;
pub mod repositorio;

pub use almacen_local::AlmacenLocal;
pub use reloj::RelojFijo;
pub use repositorio::RepositorioEnMemoria;
