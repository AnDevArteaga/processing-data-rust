//! El worker: toma jobs de la cola, corre el motor y liquida el resultado.
//!
//! Es una librería y un binario. La API puede embeber el bucle en el mismo
//! proceso; `dp-worker` abre el mismo SQLite y drena la cola por separado.

use chrono::Duration;
use dp_core::{ErrorDp, Sumidero, SumideroCsv, SumideroJson, SumideroNulo, buscar_operacion};
use dp_dominio::{
    Almacen, Archivo, ErrorAlmacen, ErrorDelJob, ErrorRepositorio, EstadoArchivo, EstadoJob,
    FormatoSalida, IdJob, Job, LibroCreditos, Reloj, RepositorioArchivos, RepositorioJobs,
    TipoArchivo, TransicionInvalida, creditos_reales,
};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration as DuracionStd;
use thiserror::Error;
use tokio::sync::watch;

/// Cuánto tiempo puede tener un worker un job antes de que otro lo rescate.
/// Tiene que ser mayor que el job más largo del plan más alto (1 hora) más
/// un colchón para escribir la salida.
const PLAZO_RECLAMACION_SEGS: i64 = 3_900;

#[derive(Debug, Error)]
pub enum ErrorWorker {
    #[error(transparent)]
    Repositorio(#[from] ErrorRepositorio),

    #[error(transparent)]
    Almacen(#[from] ErrorAlmacen),

    #[error(transparent)]
    Transicion(#[from] TransicionInvalida),

    #[error("fallo de entrada/salida: {0}")]
    Io(#[from] std::io::Error),
}

/// Todo lo que el worker necesita del mundo. Los campos son puertos: el
/// mismo código corre contra memoria en los tests y contra Postgres mañana.
pub struct Contexto {
    pub id: String,
    pub jobs: Arc<dyn RepositorioJobs>,
    pub archivos: Arc<dyn RepositorioArchivos>,
    pub almacen: Arc<dyn Almacen>,
    pub reloj: Arc<dyn Reloj>,
    pub libro: Arc<dyn LibroCreditos>,
    pub intervalo_poll: DuracionStd,
    pub intervalo_rescate: DuracionStd,
    pub intervalo_limpieza: DuracionStd,
}

impl Contexto {
    pub fn nuevo(
        id: impl Into<String>,
        jobs: Arc<dyn RepositorioJobs>,
        archivos: Arc<dyn RepositorioArchivos>,
        almacen: Arc<dyn Almacen>,
        reloj: Arc<dyn Reloj>,
        libro: Arc<dyn LibroCreditos>,
    ) -> Self {
        Contexto {
            id: id.into(),
            jobs,
            archivos,
            almacen,
            reloj,
            libro,
            intervalo_poll: DuracionStd::from_millis(250),
            intervalo_rescate: DuracionStd::from_secs(5),
            intervalo_limpieza: DuracionStd::from_secs(60),
        }
    }
}

/// Bucle del worker. Sale cuando `parada` pasa a `true`.
pub async fn arrancar(ctx: Contexto, mut parada: watch::Receiver<bool>) {
    let mut poll = tokio::time::interval(ctx.intervalo_poll);
    let mut rescate = tokio::time::interval(ctx.intervalo_rescate);
    let mut limpieza = tokio::time::interval(ctx.intervalo_limpieza);
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    rescate.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    limpieza.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    tracing::info!(worker = %ctx.id, "worker en marcha");

    loop {
        tokio::select! {
            _ = parada.changed() => {
                if *parada.borrow() {
                    tracing::info!(worker = %ctx.id, "worker deteniendose");
                    break;
                }
            }
            _ = poll.tick() => {
                if let Err(error) = drenar(&ctx).await {
                    tracing::error!(%error, "fallo al drenar la cola");
                }
            }
            _ = rescate.tick() => {
                match ctx.jobs.rescatar_vencidos(ctx.reloj.ahora()).await {
                    Ok(0) => {}
                    Ok(n) => tracing::warn!(rescatados = n, "jobs de workers perdidos reencolados"),
                    Err(error) => tracing::error!(%error, "fallo al rescatar jobs vencidos"),
                }
            }
            _ = limpieza.tick() => {
                match limpiar_vencidos(&ctx).await {
                    Ok(0) => {}
                    Ok(n) => tracing::info!(borrados = n, "archivos vencidos eliminados"),
                    Err(error) => tracing::error!(%error, "fallo al limpiar archivos vencidos"),
                }
            }
        }
    }
}

/// Procesa todos los jobs encolados hasta vaciar la cola.
pub async fn drenar(ctx: &Contexto) -> Result<u64, ErrorWorker> {
    let mut procesados = 0;
    loop {
        match procesar_siguiente(ctx).await {
            Ok(Some(_)) => procesados += 1,
            Ok(None) => return Ok(procesados),
            Err(error) => return Err(error),
        }
    }
}

/// Reclama un job, lo ejecuta y persiste el desenlace. `None` si no había nada.
pub async fn procesar_siguiente(ctx: &Contexto) -> Result<Option<IdJob>, ErrorWorker> {
    let ahora = ctx.reloj.ahora();
    let plazo = Duration::seconds(PLAZO_RECLAMACION_SEGS);

    let Some(mut job) = ctx.jobs.reclamar_siguiente(&ctx.id, plazo, ahora).await? else {
        return Ok(None);
    };

    let id = job.id;
    tracing::info!(
        job = %id,
        operacion = %job.operacion,
        intento = job.intentos,
        "job reclamado"
    );

    if let Err(error) = ejecutar_job(ctx, &mut job).await {
        // Red de seguridad: si fallamos antes de completar o fallar el job,
        // no lo dejamos congelado en PROCESANDO.
        if !job.estado.es_final() && job.estado == dp_dominio::EstadoJob::Procesando {
            let _ = job.fallar(
                ErrorDelJob::interno("E_WORKER", error.to_string()),
                ctx.reloj.ahora(),
            );
        }
        tracing::error!(job = %id, %error, "el worker no pudo terminar el job");
    }

    let estado_final = job.estado;
    let org = job.organizacion;
    let cobrado = job.creditos_cobrados.unwrap_or(0);
    ctx.jobs.guardar(job).await?;
    liquidar_creditos(ctx, estado_final, org, id, cobrado).await?;
    Ok(Some(id))
}

async fn liquidar_creditos(
    ctx: &Contexto,
    estado: EstadoJob,
    org: dp_dominio::IdOrganizacion,
    job: IdJob,
    cobrado: u64,
) -> Result<(), ErrorWorker> {
    let ahora = ctx.reloj.ahora();
    match estado {
        EstadoJob::Completado => ctx.libro.confirmar(org, job, cobrado, ahora).await?,
        EstadoJob::Fallido | EstadoJob::Cancelado | EstadoJob::SinSalida => {
            ctx.libro.liberar(org, job, ahora).await?;
        }
        _ => {}
    }
    Ok(())
}

async fn ejecutar_job(ctx: &Contexto, job: &mut Job) -> Result<(), ErrorWorker> {
    let ahora = ctx.reloj.ahora();

    let archivo = match ctx.archivos.obtener(job.organizacion, job.entrada).await? {
        Some(archivo) if archivo.esta_disponible() => archivo,
        Some(_) | None => {
            job.fallar(
                ErrorDelJob::del_cliente(
                    "E_ARCHIVO_SIN_CONTENIDO",
                    "el archivo de entrada no existe o ya no tiene contenido",
                ),
                ahora,
            )?;
            return Ok(());
        }
    };

    let ruta_entrada = ctx.almacen.materializar(&archivo.clave).await?;
    let config = job.opciones.a_config(&job.limites);
    let Some(operacion) = buscar_operacion(&job.operacion, config) else {
        job.fallar(
            ErrorDelJob::del_cliente(
                "E_OPERACION_DESCONOCIDA",
                format!("la operacion '{}' ya no existe", job.operacion),
            ),
            ahora,
        )?;
        return Ok(());
    };

    let produce = operacion.produce_archivo();
    let formato = job.opciones.formato;
    let dir_tmp = directorio_temporal(job.id);
    tokio::fs::create_dir_all(&dir_tmp).await?;
    let ruta_salida = dir_tmp.join(format!("salida.{}", formato.extension()));

    let entrada_str = ruta_entrada.to_string_lossy().into_owned();
    let salida_str = ruta_salida.to_string_lossy().into_owned();
    let timeout = DuracionStd::from_secs(job.limites.segundos_por_job);

    let resultado = tokio::time::timeout(
        timeout,
        tokio::task::spawn_blocking(move || -> Result<dp_core::Resumen, ErrorDp> {
            let mut sumidero: Box<dyn Sumidero> = if !produce {
                Box::new(SumideroNulo)
            } else if salida_str.ends_with(".json") {
                Box::new(SumideroJson::nuevo(&salida_str)?)
            } else {
                Box::new(SumideroCsv::nuevo(&salida_str)?)
            };
            operacion.ejecutar(&entrada_str, &mut *sumidero)
        }),
    )
    .await;

    let ahora = ctx.reloj.ahora();
    let desenlace = match resultado {
        Err(_) => {
            job.fallar(
                ErrorDelJob::interno(
                    "E_TIEMPO_AGOTADO",
                    format!(
                        "el job supero el limite de {} segundos del plan",
                        job.limites.segundos_por_job
                    ),
                ),
                ahora,
            )?;
            tracing::warn!(job = %job.id, "job abortado por timeout");
            None
        }
        Ok(Err(join)) => {
            job.fallar(
                ErrorDelJob::interno("E_WORKER", format!("el hilo del motor se cayo: {join}")),
                ahora,
            )?;
            None
        }
        Ok(Ok(Err(error))) => {
            let clasificado = ErrorDelJob::desde_motor(&error);
            tracing::info!(
                job = %job.id,
                codigo = %clasificado.codigo,
                culpa_del_cliente = clasificado.culpa_del_cliente,
                "el motor rechazo el job"
            );
            job.fallar(clasificado, ahora)?;
            None
        }
        Ok(Ok(Ok(resumen))) => Some(resumen),
    };

    if let Some(resumen) = desenlace {
        match publicar_si_hace_falta(ctx, job, produce, &archivo, &ruta_salida, formato).await {
            Ok(salida) => {
                let cobrados = creditos_reales(&job.operacion, &resumen, job.opciones.deduplica());
                job.completar(salida, resumen, cobrados, ahora)?;
                tracing::info!(
                    job = %job.id,
                    leidas = resumen.leidas,
                    escritas = resumen.escritas,
                    creditos = cobrados,
                    "job completado"
                );
            }
            Err(error) => {
                job.fallar(ErrorDelJob::interno("E_ALMACEN", error.to_string()), ahora)?;
            }
        }
    }

    let _ = tokio::fs::remove_dir_all(dir_tmp).await;
    Ok(())
}

async fn publicar_si_hace_falta(
    ctx: &Contexto,
    job: &Job,
    produce: bool,
    entrada: &Archivo,
    ruta_salida: &Path,
    formato: FormatoSalida,
) -> Result<Option<dp_dominio::IdArchivo>, ErrorWorker> {
    if !produce {
        return Ok(None);
    }

    let (bytes, huella) = huella_de(ruta_salida).await?;
    let tipo = match formato {
        FormatoSalida::Csv => TipoArchivo::Csv,
        FormatoSalida::Json => TipoArchivo::Json,
    };

    let mut salida = Archivo::pendiente(
        job.organizacion,
        &nombre_salida(&entrada.nombre_original, formato),
        ctx.reloj.ahora(),
    );
    ctx.almacen.subir(&salida.clave, ruta_salida).await?;
    salida.confirmar(bytes, tipo, huella);
    let id = salida.id;
    ctx.archivos.crear(salida).await?;
    Ok(Some(id))
}

async fn huella_de(ruta: &Path) -> Result<(u64, String), ErrorWorker> {
    let contenido = tokio::fs::read(ruta).await?;
    let mut hasher = Sha256::new();
    hasher.update(&contenido);
    Ok((contenido.len() as u64, hex::encode(hasher.finalize())))
}

fn nombre_salida(original: &str, formato: FormatoSalida) -> String {
    let stem = Path::new(original)
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("salida");
    format!("{stem}.clean.{}", formato.extension())
}

fn directorio_temporal(id: IdJob) -> PathBuf {
    std::env::temp_dir().join(format!("dp-job-{id}"))
}

/// Borra del almacén los archivos cuya retención venció y los marca.
pub async fn limpiar_vencidos(ctx: &Contexto) -> Result<u64, ErrorWorker> {
    let ahora = ctx.reloj.ahora();
    let vencidos = ctx.archivos.vencidos(ahora).await?;
    let mut borrados = 0;

    for mut archivo in vencidos {
        if archivo.estado == EstadoArchivo::Vencido {
            continue;
        }
        ctx.almacen.borrar(&archivo.clave).await?;
        archivo.vencer();
        ctx.archivos.actualizar(archivo).await?;
        borrados += 1;
    }

    Ok(borrados)
}

/// Identificador de este proceso, para el campo `reclamado_por`.
pub fn id_de_este_proceso() -> String {
    std::env::var("DP_WORKER_ID").unwrap_or_else(|_| format!("worker-{}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use dp_core::Resumen;
    use dp_dominio::{
        IdOrganizacion, LibroCreditos, OpcionesJob, Organizacion, Plan, RETENCION_HORAS,
        RepositorioCuentas,
    };
    use dp_memoria::{AlmacenLocal, RelojFijo, RepositorioEnMemoria};

    fn ruta_datos(nombre: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../data")
            .join(nombre)
    }

    async fn fixture(etiqueta: &str) -> (Contexto, Arc<RepositorioEnMemoria>, PathBuf) {
        let raiz = std::env::temp_dir().join(format!(
            "dp-worker-{etiqueta}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let repo = Arc::new(RepositorioEnMemoria::nuevo());
        let almacen = Arc::new(AlmacenLocal::nuevo(&raiz).await.unwrap());
        let reloj: Arc<dyn Reloj> = Arc::new(RelojFijo::default());
        let ctx = Contexto::nuevo(
            "worker-test",
            repo.clone(),
            repo.clone(),
            almacen,
            reloj,
            repo.clone(),
        );
        (ctx, repo, raiz)
    }

    async fn organizar(repo: &RepositorioEnMemoria, saldo: u64) -> IdOrganizacion {
        let ahora = Utc::now();
        let org = Organizacion::nueva("test", Plan::Pro, ahora);
        let id = org.id;
        repo.crear_organizacion(org).await.unwrap();
        if saldo > 0 {
            repo.acreditar(id, saldo, "asignacion de prueba", ahora)
                .await
                .unwrap();
        }
        id
    }

    async fn encolar_csv(
        ctx: &Contexto,
        org: IdOrganizacion,
        nombre: &str,
        operacion: &str,
        opciones: OpcionesJob,
        limites: dp_dominio::Limites,
    ) -> IdJob {
        let bytes = tokio::fs::read(ruta_datos(nombre)).await.unwrap();
        let mut archivo = Archivo::pendiente(org, nombre, ctx.reloj.ahora());
        ctx.almacen.guardar(&archivo.clave, &bytes).await.unwrap();
        archivo.confirmar(bytes.len() as u64, TipoArchivo::Csv, "test".to_string());
        let entrada = archivo.id;
        ctx.archivos.crear(archivo).await.unwrap();

        let ahora = ctx.reloj.ahora();
        let job = Job::nuevo(org, operacion, entrada, opciones, limites, 5, ahora);
        let id = job.id;
        ctx.libro.reservar(org, id, 5, ahora).await.unwrap();
        ctx.jobs.crear(job).await.unwrap();
        id
    }

    async fn job_de(ctx: &Contexto, org: IdOrganizacion, id: IdJob) -> Job {
        ctx.jobs.obtener(org, id).await.unwrap().unwrap()
    }

    #[tokio::test]
    async fn limpia_el_csv_de_prueba_y_publica_la_salida() {
        let (ctx, repo, raiz) = fixture("clean").await;
        let org = organizar(&repo, 1_000).await;
        let id = encolar_csv(
            &ctx,
            org,
            "clientes.csv",
            "csv.clean",
            OpcionesJob::default(),
            Plan::Pro.limites(),
        )
        .await;

        assert_eq!(procesar_siguiente(&ctx).await.unwrap(), Some(id));

        let job = job_de(&ctx, org, id).await;
        assert_eq!(job.estado, dp_dominio::EstadoJob::Completado);
        assert_eq!(
            job.resumen,
            Some(Resumen {
                leidas: 15,
                escritas: 12,
                duplicadas: 3,
                emails_invalidos: 2,
                telefonos_invalidos: 1,
                telefonos_vacios: 1,
                campos_normalizados: 5,
            })
        );
        assert_eq!(job.creditos_cobrados, Some(1));
        assert!(job.salida.is_some());
        assert_eq!(repo.saldo(org).await.unwrap(), 999);

        let salida = ctx
            .archivos
            .obtener(org, job.salida.unwrap())
            .await
            .unwrap()
            .unwrap();
        assert!(salida.esta_disponible());
        assert_eq!(salida.nombre_original, "clientes.clean.csv");
        assert!(ctx.almacen.existe(&salida.clave).await.unwrap());

        tokio::fs::remove_dir_all(raiz).await.ok();
    }

    #[tokio::test]
    async fn un_csv_sin_columna_falla_sin_reintentar() {
        let (ctx, repo, raiz) = fixture("columna").await;
        let org = organizar(&repo, 1_000).await;
        let id = encolar_csv(
            &ctx,
            org,
            "sin_columna.csv",
            "csv.clean",
            OpcionesJob::default(),
            Plan::Pro.limites(),
        )
        .await;

        procesar_siguiente(&ctx).await.unwrap();

        let job = job_de(&ctx, org, id).await;
        assert_eq!(job.estado, dp_dominio::EstadoJob::Fallido);
        assert_eq!(job.error.as_ref().unwrap().codigo, "E_COLUMNA_FALTANTE");
        assert!(job.error.as_ref().unwrap().culpa_del_cliente);
        assert!(job.salida.is_none());

        tokio::fs::remove_dir_all(raiz).await.ok();
    }

    #[tokio::test]
    async fn inspeccionar_no_publica_archivo() {
        let (ctx, repo, raiz) = fixture("inspect").await;
        let org = organizar(&repo, 1_000).await;
        let id = encolar_csv(
            &ctx,
            org,
            "clientes.csv",
            "csv.inspect",
            OpcionesJob::default(),
            Plan::Pro.limites(),
        )
        .await;

        procesar_siguiente(&ctx).await.unwrap();

        let job = job_de(&ctx, org, id).await;
        assert_eq!(job.estado, dp_dominio::EstadoJob::Completado);
        assert!(job.salida.is_none());
        assert_eq!(job.resumen.unwrap().escritas, 0);

        tokio::fs::remove_dir_all(raiz).await.ok();
    }

    #[tokio::test]
    async fn sin_jobs_en_cola_no_hace_nada() {
        let (ctx, _, raiz) = fixture("vacio").await;
        assert!(procesar_siguiente(&ctx).await.unwrap().is_none());
        tokio::fs::remove_dir_all(raiz).await.ok();
    }

    #[tokio::test]
    async fn la_limpieza_borra_bytes_y_marca_vencido() {
        let raiz = std::env::temp_dir().join(format!(
            "dp-worker-limpieza-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let repo = Arc::new(RepositorioEnMemoria::nuevo());
        let almacen = Arc::new(AlmacenLocal::nuevo(&raiz).await.unwrap());
        let inicio = Utc::now();
        let reloj = Arc::new(RelojFijo::nuevo(inicio));
        let ctx = Contexto::nuevo(
            "w",
            repo.clone(),
            repo.clone(),
            almacen.clone(),
            reloj.clone(),
            repo.clone(),
        );

        let org = IdOrganizacion::nuevo();
        let mut archivo = Archivo::pendiente(org, "viejo.csv", inicio);
        almacen
            .guardar(&archivo.clave, b"nombre,email\n")
            .await
            .unwrap();
        archivo.confirmar(14, TipoArchivo::Csv, "x".to_string());
        let clave = archivo.clave.clone();
        ctx.archivos.crear(archivo).await.unwrap();

        assert_eq!(limpiar_vencidos(&ctx).await.unwrap(), 0);
        assert!(almacen.existe(&clave).await.unwrap());

        reloj.adelantar(Duration::hours(RETENCION_HORAS));
        assert_eq!(limpiar_vencidos(&ctx).await.unwrap(), 1);
        assert!(!almacen.existe(&clave).await.unwrap());

        // La segunda pasada no vuelve a contar lo ya vencido.
        assert_eq!(limpiar_vencidos(&ctx).await.unwrap(), 0);

        tokio::fs::remove_dir_all(raiz).await.ok();
    }
}
