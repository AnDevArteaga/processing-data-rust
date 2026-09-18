//! La API v1.

pub mod auth;
pub mod dto;
pub mod error;
pub mod estado;
pub mod firma;
pub mod rutas;

pub use error::ErrorApi;
pub use estado::Estado;
pub use rutas::enrutador;

use auth::{AutenticadorCuentas, AutenticadorFijo};
use dp_dominio::{ApiKey, LibroCreditos, Organizacion, Plan, RelojDelSistema, RepositorioCuentas};
use dp_memoria::{AlmacenLocal, RepositorioEnMemoria};
use dp_persistencia::RepositorioSqlite;
use firma::Firmante;
use std::sync::Arc;

pub struct Config {
    pub direccion: String,
    pub base_publica: String,
    pub directorio_almacen: String,
    pub base_datos: String,
    pub secreto_firma: String,
    pub token_dev: String,
    pub plan_dev: Plan,
    /// Si es true, no toca disco: sirve para tests. En producción es false.
    pub en_memoria: bool,
}

impl Config {
    pub fn desde_entorno() -> Self {
        let puerto = variable("DP_PUERTO", "8080");
        Config {
            direccion: format!("0.0.0.0:{puerto}"),
            base_publica: variable("DP_BASE_PUBLICA", &format!("http://localhost:{puerto}")),
            directorio_almacen: variable("DP_ALMACEN", "data/almacen"),
            base_datos: variable("DP_DATABASE", "data/plataforma.sqlite"),
            secreto_firma: variable("DP_SECRETO_FIRMA", "secreto-de-desarrollo-cambiame"),
            token_dev: variable("DP_TOKEN", "dp_dev_token"),
            plan_dev: Plan::Pro,
            en_memoria: std::env::var("DP_MEMORIA").ok().as_deref() == Some("1"),
        }
    }
}

fn variable(nombre: &str, por_defecto: &str) -> String {
    std::env::var(nombre).unwrap_or_else(|_| por_defecto.to_string())
}

pub async fn montar(config: &Config) -> Result<Arc<Estado>, ErrorApi> {
    let almacen = Arc::new(AlmacenLocal::nuevo(&config.directorio_almacen).await?);
    let reloj = Arc::new(RelojDelSistema);
    let firmante = Arc::new(Firmante::nuevo(&config.secreto_firma));

    if config.en_memoria {
        return montar_memoria(config, almacen, reloj, firmante).await;
    }

    let repo = Arc::new(RepositorioSqlite::abrir(&config.base_datos).await?);
    let semilla = sembrar_si_vacio(repo.as_ref(), config).await?;

    if let Some((org, token)) = &semilla {
        tracing::info!(organizacion = %org.id, plan = org.plan.etiqueta(), "organizacion inicial creada");
        tracing::info!(token = %token, "API key inicial (guardala: no se vuelve a mostrar)");
    }

    Ok(Arc::new(Estado {
        archivos: repo.clone(),
        jobs: repo.clone(),
        cuentas: repo.clone(),
        libro: repo.clone(),
        almacen,
        autenticador: Arc::new(AutenticadorCuentas::nuevo(repo)),
        reloj,
        firmante,
        base_publica: config.base_publica.clone(),
    }))
}

async fn montar_memoria(
    config: &Config,
    almacen: Arc<AlmacenLocal>,
    reloj: Arc<RelojDelSistema>,
    firmante: Arc<Firmante>,
) -> Result<Arc<Estado>, ErrorApi> {
    let repo = Arc::new(RepositorioEnMemoria::nuevo());
    let ahora = chrono::Utc::now();
    let org = Organizacion::nueva("Desarrollo", config.plan_dev, ahora);
    let organizacion = org.id;
    repo.crear_organizacion(org).await?;
    repo.acreditar(
        organizacion,
        config.plan_dev.limites().creditos_mensuales,
        "asignacion inicial del plan",
        ahora,
    )
    .await?;

    tracing::info!(organizacion = %organizacion, "organizacion de desarrollo en memoria");

    Ok(Arc::new(Estado {
        archivos: repo.clone(),
        jobs: repo.clone(),
        cuentas: repo.clone(),
        libro: repo.clone(),
        almacen,
        autenticador: Arc::new(AutenticadorFijo::nuevo(
            &config.token_dev,
            organizacion,
            config.plan_dev,
        )),
        reloj,
        firmante,
        base_publica: config.base_publica.clone(),
    }))
}

async fn sembrar_si_vacio(
    cuentas: &RepositorioSqlite,
    config: &Config,
) -> Result<Option<(Organizacion, String)>, ErrorApi> {
    // Si ya hay una org, no tocamos nada: un arranque posterior no rota la key.
    if cuentas.alguna_organizacion().await?.is_some() {
        return Ok(None);
    }

    let ahora = chrono::Utc::now();
    let org = Organizacion::nueva("Desarrollo", config.plan_dev, ahora);
    let id = org.id;
    cuentas.crear_organizacion(org.clone()).await?;
    cuentas
        .acreditar(
            id,
            config.plan_dev.limites().creditos_mensuales,
            "asignacion inicial del plan",
            ahora,
        )
        .await?;
    let (clave, token) = ApiKey::emitir(id, "inicial", ahora);
    cuentas.crear_api_key(clave).await?;
    Ok(Some((org, token)))
}

pub fn contexto_del_worker(estado: &Estado) -> dp_worker::Contexto {
    dp_worker::Contexto::nuevo(
        dp_worker::id_de_este_proceso(),
        estado.jobs.clone(),
        estado.archivos.clone(),
        estado.almacen.clone(),
        estado.reloj.clone(),
        estado.libro.clone(),
    )
}
