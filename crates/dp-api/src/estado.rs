//! El estado compartido del servidor.
//!
//! Aquí se ve la inversión de dependencias hecha realidad: el servidor guarda
//! `Arc<dyn RepositorioJobs>`, no un `RepositorioPostgres`. Cambiar de
//! PostgreSQL a otra cosa es construir el `Estado` con otro argumento.

use crate::auth::Autenticador;
use crate::firma::Firmante;
use dp_dominio::{
    Almacen, LibroCreditos, Reloj, RepositorioArchivos, RepositorioCuentas, RepositorioJobs,
};
use std::sync::Arc;

pub struct Estado {
    pub archivos: Arc<dyn RepositorioArchivos>,
    pub jobs: Arc<dyn RepositorioJobs>,
    pub cuentas: Arc<dyn RepositorioCuentas>,
    pub libro: Arc<dyn LibroCreditos>,
    pub almacen: Arc<dyn Almacen>,
    pub autenticador: Arc<dyn Autenticador>,
    pub reloj: Arc<dyn Reloj>,
    pub firmante: Arc<Firmante>,

    /// La base pública del servidor, para construir las URLs temporales.
    /// Detrás de un proxy inverso esto no es lo mismo que el puerto donde
    /// escucha, así que tiene que ser configurable.
    pub base_publica: String,
}
