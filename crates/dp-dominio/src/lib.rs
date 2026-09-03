//! El dominio de la plataforma: qué es un job, un archivo, un plan y un
//! crédito, más los puertos por los que entra la infraestructura.
//!
//! La regla que sostiene toda la arquitectura está aquí: **este crate no
//! depende de ninguna infraestructura**. No sabe qué es PostgreSQL, ni S3, ni
//! HTTP. Solo depende de `dp-core` para reusar el `Resumen` y los errores
//! clasificados del motor.
//!
//! La dirección de las dependencias es lo único que importa:
//!
//! ```text
//! dp-api ──┐
//!          ├──> dp-dominio ──> dp-core
//! dp-worker┘         ▲
//!                    │
//!              dp-memoria (implementa los puertos)
//! ```
//!
//! Todas las flechas apuntan hacia adentro. Si algún día alguien escribe
//! `use sqlx::...` en este crate, la arquitectura se rompió.

pub mod archivo;
pub mod creditos;
pub mod error;
pub mod ids;
pub mod job;
pub mod opciones;
pub mod plan;
pub mod puertos;

pub use archivo::{Archivo, EstadoArchivo, TipoArchivo};
pub use creditos::{creditos_estimados, creditos_reales};
pub use error::{ErrorAlmacen, ErrorRepositorio, TransicionInvalida};
pub use ids::{ErrorId, IdApiKey, IdArchivo, IdJob, IdOrganizacion};
pub use job::{Desenlace, ErrorDelJob, EstadoJob, Job};
pub use opciones::{FormatoSalida, OpcionesJob};
pub use plan::{Limites, Plan};
pub use puertos::{Almacen, Reloj, RelojDelSistema, RepositorioArchivos, RepositorioJobs};
