//! Binario autónomo del worker.
//!
//! Comparte el mismo archivo SQLite y el mismo directorio de almacén que
//! `dp-api`. Arranca la API con `DP_SIN_WORKER=1` si este proceso es quien
//! drena la cola.

use dp_dominio::RelojDelSistema;
use dp_memoria::AlmacenLocal;
use dp_persistencia::RepositorioSqlite;
use dp_worker::{Contexto, arrancar, id_de_este_proceso};
use std::sync::Arc;
use tokio::sync::watch;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "dp_worker=info".into()),
        )
        .init();

    let base_datos = variable("DP_DATABASE", "data/plataforma.sqlite");
    let directorio_almacen = variable("DP_ALMACEN", "data/almacen");

    let repo = match RepositorioSqlite::abrir(&base_datos).await {
        Ok(repo) => Arc::new(repo),
        Err(error) => {
            tracing::error!(%error, ruta = %base_datos, "no pude abrir la base de datos");
            std::process::exit(1);
        }
    };
    let almacen = match AlmacenLocal::nuevo(&directorio_almacen).await {
        Ok(almacen) => Arc::new(almacen),
        Err(error) => {
            tracing::error!(%error, ruta = %directorio_almacen, "no pude abrir el almacen");
            std::process::exit(1);
        }
    };

    let ctx = Contexto::nuevo(
        id_de_este_proceso(),
        repo.clone(),
        repo.clone(),
        almacen,
        Arc::new(RelojDelSistema),
        repo,
    );

    tracing::info!(base_datos = %base_datos, almacen = %directorio_almacen, worker = %ctx.id, "worker autonomo en marcha");

    let (parar_tx, parar_rx) = watch::channel(false);
    tokio::spawn(async move {
        match tokio::signal::ctrl_c().await {
            Ok(()) => {
                tracing::info!("cierre solicitado");
                let _ = parar_tx.send(true);
            }
            Err(error) => tracing::error!(%error, "no pude escuchar la senal de cierre"),
        }
    });

    arrancar(ctx, parar_rx).await;
}

fn variable(nombre: &str, por_defecto: &str) -> String {
    std::env::var(nombre).unwrap_or_else(|_| por_defecto.to_string())
}
