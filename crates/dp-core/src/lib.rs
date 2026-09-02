//! Núcleo de procesamiento de datos.
//!
//! Esta es la librería reutilizable: la CLI de hoy y el worker que consumirá
//! la cola de Redis mañana dependen de ella sin duplicar nada.

pub mod error;
pub mod modelo;
pub mod normalizacion;
pub mod operaciones;
pub mod sumidero;

// Re-exportamos lo más usado en la raíz para que quien consuma la librería
// escriba `dp::Resumen` en vez de `dp::modelo::Resumen`.
pub use error::{ErrorDp, Resultado};
pub use modelo::{ClienteBruto, ClienteLimpio, EstadoEmail, Resumen};
pub use operaciones::{Operacion, buscar_operacion, catalogo};
pub use sumidero::{Sumidero, SumideroCsv, SumideroJson, SumideroNulo};
