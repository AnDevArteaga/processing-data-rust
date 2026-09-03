//! La API v1.
//!
//! Es una librería y no solo un binario a propósito: así los tests de
//! integración construyen el enrutador completo, con repositorio en memoria y
//! almacén temporal, y le mandan peticiones sin abrir un puerto ni levantar
//! una base de datos. La suite entera corre en milisegundos.

pub mod auth;
pub mod dto;
pub mod error;
pub mod estado;
pub mod firma;
pub mod rutas;

pub use error::ErrorApi;
pub use estado::Estado;
pub use rutas::enrutador;

use auth::AutenticadorFijo;
use dp_dominio::{Plan, RelojDelSistema};
use dp_memoria::{AlmacenLocal, RepositorioEnMemoria};
use firma::Firmante;
use std::sync::Arc;

/// La configuración del proceso, leída del entorno.
///
/// Las variables se leen UNA vez al arrancar y no en cada petición: un
/// servidor que decide su comportamiento leyendo el entorno a mitad de vuelo
/// es imposible de razonar.
pub struct Config {
    pub direccion: String,
    pub base_publica: String,
    pub directorio_almacen: String,
    pub secreto_firma: String,
    pub token_dev: String,
    pub plan_dev: Plan,
}

impl Config {
    pub fn desde_entorno() -> Self {
        let puerto = variable("DP_PUERTO", "8080");
        Config {
            direccion: format!("0.0.0.0:{puerto}"),
            // Detrás de un proxy inverso la base pública no es la dirección
            // donde escuchamos, así que es una variable aparte.
            base_publica: variable("DP_BASE_PUBLICA", &format!("http://localhost:{puerto}")),
            directorio_almacen: variable("DP_ALMACEN", "data/almacen"),
            secreto_firma: variable("DP_SECRETO_FIRMA", "secreto-de-desarrollo-cambiame"),
            token_dev: variable("DP_TOKEN", "dp_dev_token"),
            plan_dev: Plan::Pro,
        }
    }
}

fn variable(nombre: &str, por_defecto: &str) -> String {
    std::env::var(nombre).unwrap_or_else(|_| por_defecto.to_string())
}

/// Monta el estado completo con las implementaciones de la fase 1.
///
/// Esta función es el único sitio donde se decide qué implementación de cada
/// puerto se usa. En la fase 2, cambiar `RepositorioEnMemoria` por
/// `RepositorioPostgres` es una línea aquí y nada más.
pub async fn montar(config: &Config) -> Result<Arc<Estado>, ErrorApi> {
    let repositorio = Arc::new(RepositorioEnMemoria::nuevo());
    let almacen = Arc::new(AlmacenLocal::nuevo(&config.directorio_almacen).await?);
    let organizacion = dp_dominio::IdOrganizacion::nuevo();

    tracing::info!(
        organizacion = %organizacion,
        plan = config.plan_dev.etiqueta(),
        "organizacion de desarrollo creada"
    );

    Ok(Arc::new(Estado {
        // El mismo objeto sirve como los dos repositorios. `Arc::clone` no
        // copia nada: solo incrementa un contador.
        archivos: repositorio.clone(),
        jobs: repositorio,
        almacen,
        autenticador: Arc::new(AutenticadorFijo::nuevo(
            &config.token_dev,
            organizacion,
            config.plan_dev,
        )),
        reloj: Arc::new(RelojDelSistema),
        firmante: Arc::new(Firmante::nuevo(&config.secreto_firma)),
        base_publica: config.base_publica.clone(),
    }))
}
