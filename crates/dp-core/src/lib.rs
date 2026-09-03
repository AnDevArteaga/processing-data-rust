//! Núcleo de procesamiento de datos.
//!
//! Esta es la librería reutilizable: la CLI de hoy y el worker que consumirá
//! la cola de Redis mañana dependen de ella sin duplicar nada.

pub mod error;
pub mod fila;
pub mod limpieza;
pub mod modelo;
pub mod normalizacion;
pub mod operaciones;
pub mod salida;
pub mod sumidero;

// Re-exportamos lo más usado en la raíz para que quien consuma la librería
// escriba `dp_core::Resumen` en vez de `dp_core::modelo::Resumen`.
pub use error::{ErrorDp, Resultado};
pub use fila::Encabezados;
pub use limpieza::{Config, LIMITE_CLAVES_POR_DEFECTO, Limpiador, SEPARADOR_CLAVE};
pub use modelo::{EstadoEmail, Resumen};
pub use operaciones::{Operacion, buscar_operacion, catalogo};
pub use salida::{DetalleError, Envoltura};
pub use sumidero::{Sumidero, SumideroCsv, SumideroJson, SumideroMemoria, SumideroNulo};
