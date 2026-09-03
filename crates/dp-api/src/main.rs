//! Arranque del servidor. Lo único que hace es cablear, lanzar el worker y escuchar.

use dp_api::{Config, contexto_del_worker, enrutador, montar};
use tokio::sync::watch;

#[tokio::main]
async fn main() {
    // Logs estructurados con filtro por variable de entorno. `RUST_LOG=debug`
    // sube el detalle sin recompilar.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "dp_api=info,dp_worker=info,tower_http=info".into()),
        )
        .init();

    let config = Config::desde_entorno();

    let estado = match montar(&config).await {
        Ok(estado) => estado,
        Err(error) => {
            tracing::error!(%error, "no pude montar el estado del servidor");
            std::process::exit(1);
        }
    };

    // El worker corre en este mismo proceso porque la cola es el repositorio
    // en memoria. `DP_SIN_WORKER=1` lo apaga, por si se quiere solo la API.
    let (parar_tx, parar_rx) = watch::channel(false);
    if std::env::var("DP_SIN_WORKER").ok().as_deref() != Some("1") {
        let ctx = contexto_del_worker(&estado);
        tokio::spawn(async move {
            dp_worker::arrancar(ctx, parar_rx).await;
        });
    }

    let escucha = match tokio::net::TcpListener::bind(&config.direccion).await {
        Ok(escucha) => escucha,
        Err(error) => {
            tracing::error!(direccion = %config.direccion, %error, "no pude abrir el puerto");
            std::process::exit(1);
        }
    };

    tracing::info!(
        direccion = %config.direccion,
        base_publica = %config.base_publica,
        almacen = %config.directorio_almacen,
        "API escuchando"
    );
    tracing::info!(token = %config.token_dev, "token de desarrollo");

    // `with_graceful_shutdown` es lo que hace que un despliegue no corte
    // peticiones a mitad: al recibir Ctrl+C deja de aceptar conexiones nuevas
    // y espera a que terminen las que están en curso.
    let servidor = axum::serve(escucha, enrutador(estado)).with_graceful_shutdown(cierre(parar_tx));

    if let Err(error) = servidor.await {
        tracing::error!(%error, "el servidor termino con error");
        std::process::exit(1);
    }
}

async fn cierre(parar: watch::Sender<bool>) {
    match tokio::signal::ctrl_c().await {
        Ok(()) => {
            tracing::info!("cierre solicitado, esperando peticiones en curso");
            let _ = parar.send(true);
        }
        Err(error) => tracing::error!(%error, "no pude escuchar la senal de cierre"),
    }
}
