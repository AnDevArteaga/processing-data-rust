//! Persistencia SQLite. Un archivo, WAL, y API y worker pueden abrirlo a la vez.
//!
//! No es PostgreSQL, y no pretende serlo. Es el almacén que se puede correr
//! hoy sin Docker. El esquema y los puertos son los mismos que usará Postgres:
//! cambiar el crate no toca la API ni el worker.

mod mapeo;

use crate::mapeo::{ArchivoFila, JobFila, KeyFila, MovimientoFila, OrgFila};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use dp_dominio::{
    ApiKey, Archivo, ErrorRepositorio, EstadoJob, IdApiKey, IdArchivo, IdJob, IdOrganizacion, Job,
    LibroCreditos, Movimiento, Organizacion, RepositorioArchivos, RepositorioCuentas,
    RepositorioJobs, TipoMovimiento,
};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};

pub struct RepositorioSqlite {
    pool: SqlitePool,
}

impl RepositorioSqlite {
    pub async fn abrir(ruta: impl AsRef<std::path::Path>) -> Result<Self, ErrorRepositorio> {
        let ruta = ruta.as_ref();
        if let Some(padre) = ruta.parent() {
            std::fs::create_dir_all(padre).map_err(|e| ErrorRepositorio::Interno(e.to_string()))?;
        }

        let opciones = SqliteConnectOptions::new()
            .filename(ruta)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(std::time::Duration::from_secs(5));

        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(opciones)
            .await
            .map_err(db)?;

        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .map_err(db)?;

        for sentencia in include_str!("esquema.sql")
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            sqlx::query(sentencia).execute(&pool).await.map_err(db)?;
        }

        Ok(RepositorioSqlite { pool })
    }
}

fn db(error: sqlx::Error) -> ErrorRepositorio {
    ErrorRepositorio::Interno(error.to_string())
}

#[async_trait]
impl RepositorioArchivos for RepositorioSqlite {
    async fn crear(&self, archivo: Archivo) -> Result<(), ErrorRepositorio> {
        sqlx::query(
            "INSERT INTO archivos (id, organizacion, nombre_original, clave, bytes, tipo, estado, sha256, creado_en, vence_en)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(archivo.id.to_string())
        .bind(archivo.organizacion.to_string())
        .bind(&archivo.nombre_original)
        .bind(&archivo.clave)
        .bind(archivo.bytes as i64)
        .bind(serde_json::to_string(&archivo.tipo).map_err(|e| ErrorRepositorio::Interno(e.to_string()))?)
        .bind(serde_json::to_string(&archivo.estado).map_err(|e| ErrorRepositorio::Interno(e.to_string()))?)
        .bind(&archivo.sha256)
        .bind(archivo.creado_en.to_rfc3339())
        .bind(archivo.vence_en.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(())
    }

    async fn obtener(
        &self,
        organizacion: IdOrganizacion,
        id: IdArchivo,
    ) -> Result<Option<Archivo>, ErrorRepositorio> {
        let fila = sqlx::query_as::<_, ArchivoFila>(
            "SELECT * FROM archivos WHERE id = ? AND organizacion = ?",
        )
        .bind(id.to_string())
        .bind(organizacion.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;
        fila.map(ArchivoFila::a_dominio).transpose()
    }

    async fn actualizar(&self, archivo: Archivo) -> Result<(), ErrorRepositorio> {
        let resultado = sqlx::query(
            "UPDATE archivos SET nombre_original = ?, clave = ?, bytes = ?, tipo = ?, estado = ?, sha256 = ?, vence_en = ?
             WHERE id = ?",
        )
        .bind(&archivo.nombre_original)
        .bind(&archivo.clave)
        .bind(archivo.bytes as i64)
        .bind(serde_json::to_string(&archivo.tipo).map_err(|e| ErrorRepositorio::Interno(e.to_string()))?)
        .bind(serde_json::to_string(&archivo.estado).map_err(|e| ErrorRepositorio::Interno(e.to_string()))?)
        .bind(&archivo.sha256)
        .bind(archivo.vence_en.to_rfc3339())
        .bind(archivo.id.to_string())
        .execute(&self.pool)
        .await
        .map_err(db)?;

        if resultado.rows_affected() == 0 {
            return Err(ErrorRepositorio::no_encontrado("archivo", archivo.id));
        }
        Ok(())
    }

    async fn vencidos(&self, ahora: DateTime<Utc>) -> Result<Vec<Archivo>, ErrorRepositorio> {
        let filas = sqlx::query_as::<_, ArchivoFila>("SELECT * FROM archivos WHERE vence_en <= ?")
            .bind(ahora.to_rfc3339())
            .fetch_all(&self.pool)
            .await
            .map_err(db)?;
        filas.into_iter().map(ArchivoFila::a_dominio).collect()
    }
}

#[async_trait]
impl RepositorioJobs for RepositorioSqlite {
    async fn crear(&self, job: Job) -> Result<(), ErrorRepositorio> {
        insertar_job(&self.pool, &job).await
    }

    async fn obtener(
        &self,
        organizacion: IdOrganizacion,
        id: IdJob,
    ) -> Result<Option<Job>, ErrorRepositorio> {
        let fila =
            sqlx::query_as::<_, JobFila>("SELECT * FROM jobs WHERE id = ? AND organizacion = ?")
                .bind(id.to_string())
                .bind(organizacion.to_string())
                .fetch_optional(&self.pool)
                .await
                .map_err(db)?;
        fila.map(JobFila::a_dominio).transpose()
    }

    async fn listar(
        &self,
        organizacion: IdOrganizacion,
        estado: Option<EstadoJob>,
        limite: usize,
    ) -> Result<Vec<Job>, ErrorRepositorio> {
        let filas = if let Some(estado) = estado {
            sqlx::query_as::<_, JobFila>(
                "SELECT * FROM jobs WHERE organizacion = ? AND estado = ? ORDER BY creado_en DESC LIMIT ?",
            )
            .bind(organizacion.to_string())
            .bind(estado.etiqueta())
            .bind(limite as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(db)?
        } else {
            sqlx::query_as::<_, JobFila>(
                "SELECT * FROM jobs WHERE organizacion = ? ORDER BY creado_en DESC LIMIT ?",
            )
            .bind(organizacion.to_string())
            .bind(limite as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(db)?
        };
        filas.into_iter().map(JobFila::a_dominio).collect()
    }

    async fn en_vuelo(&self, organizacion: IdOrganizacion) -> Result<usize, ErrorRepositorio> {
        let fila = sqlx::query(
            "SELECT COUNT(*) as n FROM jobs WHERE organizacion = ? AND estado IN ('queued', 'processing')",
        )
        .bind(organizacion.to_string())
        .fetch_one(&self.pool)
        .await
        .map_err(db)?;
        Ok(fila.get::<i64, _>("n") as usize)
    }

    async fn reclamar_siguiente(
        &self,
        worker: &str,
        plazo: Duration,
        ahora: DateTime<Utc>,
    ) -> Result<Option<Job>, ErrorRepositorio> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        let fila = sqlx::query_as::<_, JobFila>(
            "SELECT * FROM jobs WHERE estado = 'queued' ORDER BY creado_en ASC LIMIT 1",
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(db)?;

        let Some(fila) = fila else {
            return Ok(None);
        };

        let mut job = fila.a_dominio()?;
        job.reclamar(worker, ahora, plazo)?;
        // Solo gana quien todavía ve el job encolado. El segundo worker
        // concurrente afecta 0 filas y se va con las manos vacías.
        let ganado = actualizar_job_si(&mut *tx, &job, "queued").await?;
        if !ganado {
            tx.rollback().await.map_err(db)?;
            return Ok(None);
        }
        tx.commit().await.map_err(db)?;
        Ok(Some(job))
    }

    async fn guardar(&self, job: Job) -> Result<(), ErrorRepositorio> {
        let mut conn = self.pool.acquire().await.map_err(db)?;
        actualizar_job(&mut *conn, &job).await
    }

    async fn cancelar(
        &self,
        organizacion: IdOrganizacion,
        id: IdJob,
        ahora: DateTime<Utc>,
    ) -> Result<Job, ErrorRepositorio> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        let fila =
            sqlx::query_as::<_, JobFila>("SELECT * FROM jobs WHERE id = ? AND organizacion = ?")
                .bind(id.to_string())
                .bind(organizacion.to_string())
                .fetch_optional(&mut *tx)
                .await
                .map_err(db)?
                .ok_or_else(|| ErrorRepositorio::no_encontrado("job", id))?;

        let mut job = fila.a_dominio()?;
        job.cancelar(ahora)?;
        actualizar_job(&mut *tx, &job).await?;
        tx.commit().await.map_err(db)?;
        Ok(job)
    }

    async fn rescatar_vencidos(&self, ahora: DateTime<Utc>) -> Result<u64, ErrorRepositorio> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        let filas = sqlx::query_as::<_, JobFila>(
            "SELECT * FROM jobs WHERE estado = 'processing' AND reclamado_hasta IS NOT NULL AND reclamado_hasta <= ?",
        )
        .bind(ahora.to_rfc3339())
        .fetch_all(&mut *tx)
        .await
        .map_err(db)?;

        let mut rescatados = 0u64;
        for fila in filas {
            let mut job = fila.a_dominio()?;
            if job.rescatar(ahora).is_some() {
                actualizar_job(&mut *tx, &job).await?;
                rescatados += 1;
            }
        }
        tx.commit().await.map_err(db)?;
        Ok(rescatados)
    }
}

async fn insertar_job<'e, E>(ejecutor: E, job: &Job) -> Result<(), ErrorRepositorio>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    sqlx::query(
        "INSERT INTO jobs (
            id, organizacion, operacion, opciones, limites, entrada, salida, estado, progreso,
            intentos, max_intentos, creditos_reservados, creditos_cobrados, resumen, error,
            creado_en, iniciado_en, terminado_en, reclamado_por, reclamado_hasta
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(job.id.to_string())
    .bind(job.organizacion.to_string())
    .bind(&job.operacion)
    .bind(
        serde_json::to_string(&job.opciones)
            .map_err(|e| ErrorRepositorio::Interno(e.to_string()))?,
    )
    .bind(
        serde_json::to_string(&job.limites)
            .map_err(|e| ErrorRepositorio::Interno(e.to_string()))?,
    )
    .bind(job.entrada.to_string())
    .bind(job.salida.map(|id| id.to_string()))
    .bind(job.estado.etiqueta())
    .bind(i64::from(job.progreso))
    .bind(i64::from(job.intentos))
    .bind(i64::from(job.max_intentos))
    .bind(job.creditos_reservados as i64)
    .bind(job.creditos_cobrados.map(|n| n as i64))
    .bind(
        job.resumen
            .map(|r| serde_json::to_string(&r))
            .transpose()
            .map_err(|e| ErrorRepositorio::Interno(e.to_string()))?,
    )
    .bind(
        job.error
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| ErrorRepositorio::Interno(e.to_string()))?,
    )
    .bind(job.creado_en.to_rfc3339())
    .bind(job.iniciado_en.map(|t| t.to_rfc3339()))
    .bind(job.terminado_en.map(|t| t.to_rfc3339()))
    .bind(&job.reclamado_por)
    .bind(job.reclamado_hasta.map(|t| t.to_rfc3339()))
    .execute(ejecutor)
    .await
    .map_err(db)?;
    Ok(())
}

async fn actualizar_job<'e, E>(ejecutor: E, job: &Job) -> Result<(), ErrorRepositorio>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let ganado = actualizar_job_si(ejecutor, job, "").await?;
    if !ganado {
        return Err(ErrorRepositorio::no_encontrado("job", job.id));
    }
    Ok(())
}

/// `estado_esperado` vacío significa "cualquier estado"; si no, es un compare-and-set.
async fn actualizar_job_si<'e, E>(
    ejecutor: E,
    job: &Job,
    estado_esperado: &str,
) -> Result<bool, ErrorRepositorio>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let sql = if estado_esperado.is_empty() {
        "UPDATE jobs SET
            salida = ?, estado = ?, progreso = ?, intentos = ?, creditos_cobrados = ?,
            resumen = ?, error = ?, iniciado_en = ?, terminado_en = ?,
            reclamado_por = ?, reclamado_hasta = ?
         WHERE id = ?"
    } else {
        "UPDATE jobs SET
            salida = ?, estado = ?, progreso = ?, intentos = ?, creditos_cobrados = ?,
            resumen = ?, error = ?, iniciado_en = ?, terminado_en = ?,
            reclamado_por = ?, reclamado_hasta = ?
         WHERE id = ? AND estado = ?"
    };

    let mut q = sqlx::query(sql)
        .bind(job.salida.map(|id| id.to_string()))
        .bind(job.estado.etiqueta())
        .bind(i64::from(job.progreso))
        .bind(i64::from(job.intentos))
        .bind(job.creditos_cobrados.map(|n| n as i64))
        .bind(
            job.resumen
                .map(|r| serde_json::to_string(&r))
                .transpose()
                .map_err(|e| ErrorRepositorio::Interno(e.to_string()))?,
        )
        .bind(
            job.error
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(|e| ErrorRepositorio::Interno(e.to_string()))?,
        )
        .bind(job.iniciado_en.map(|t| t.to_rfc3339()))
        .bind(job.terminado_en.map(|t| t.to_rfc3339()))
        .bind(&job.reclamado_por)
        .bind(job.reclamado_hasta.map(|t| t.to_rfc3339()))
        .bind(job.id.to_string());
    if !estado_esperado.is_empty() {
        q = q.bind(estado_esperado);
    }
    let resultado = q.execute(ejecutor).await.map_err(db)?;
    Ok(resultado.rows_affected() > 0)
}

#[async_trait]
impl RepositorioCuentas for RepositorioSqlite {
    async fn crear_organizacion(&self, organizacion: Organizacion) -> Result<(), ErrorRepositorio> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        sqlx::query("INSERT INTO organizaciones (id, nombre, plan, creado_en) VALUES (?, ?, ?, ?)")
            .bind(organizacion.id.to_string())
            .bind(&organizacion.nombre)
            .bind(
                serde_json::to_string(&organizacion.plan)
                    .map_err(|e| ErrorRepositorio::Interno(e.to_string()))?,
            )
            .bind(organizacion.creado_en.to_rfc3339())
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        sqlx::query("INSERT INTO saldos (organizacion, disponible) VALUES (?, 0)")
            .bind(organizacion.id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        tx.commit().await.map_err(db)?;
        Ok(())
    }

    async fn alguna_organizacion(&self) -> Result<Option<Organizacion>, ErrorRepositorio> {
        let fila = sqlx::query_as::<_, OrgFila>("SELECT * FROM organizaciones LIMIT 1")
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?;
        fila.map(OrgFila::a_dominio).transpose()
    }

    async fn obtener_organizacion(
        &self,
        id: IdOrganizacion,
    ) -> Result<Option<Organizacion>, ErrorRepositorio> {
        let fila = sqlx::query_as::<_, OrgFila>("SELECT * FROM organizaciones WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?;
        fila.map(OrgFila::a_dominio).transpose()
    }

    async fn crear_api_key(&self, clave: ApiKey) -> Result<(), ErrorRepositorio> {
        sqlx::query(
            "INSERT INTO api_keys (id, organizacion, nombre, prefijo, hash, revocada, ultimo_uso, creada_en)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(clave.id.to_string())
        .bind(clave.organizacion.to_string())
        .bind(&clave.nombre)
        .bind(&clave.prefijo)
        .bind(&clave.hash)
        .bind(i64::from(u8::from(clave.revocada)))
        .bind(clave.ultimo_uso.map(|t| t.to_rfc3339()))
        .bind(clave.creada_en.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(())
    }

    async fn listar_api_keys(
        &self,
        organizacion: IdOrganizacion,
    ) -> Result<Vec<ApiKey>, ErrorRepositorio> {
        let filas = sqlx::query_as::<_, KeyFila>(
            "SELECT * FROM api_keys WHERE organizacion = ? ORDER BY creada_en DESC",
        )
        .bind(organizacion.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        filas.into_iter().map(KeyFila::a_dominio).collect()
    }

    async fn revocar_api_key(
        &self,
        organizacion: IdOrganizacion,
        id: IdApiKey,
    ) -> Result<(), ErrorRepositorio> {
        let resultado =
            sqlx::query("UPDATE api_keys SET revocada = 1 WHERE id = ? AND organizacion = ?")
                .bind(id.to_string())
                .bind(organizacion.to_string())
                .execute(&self.pool)
                .await
                .map_err(db)?;
        if resultado.rows_affected() == 0 {
            return Err(ErrorRepositorio::no_encontrado("api_key", id));
        }
        Ok(())
    }

    async fn por_hash(
        &self,
        hash: &str,
    ) -> Result<Option<(Organizacion, ApiKey)>, ErrorRepositorio> {
        let Some(clave) =
            sqlx::query_as::<_, KeyFila>("SELECT * FROM api_keys WHERE hash = ? AND revocada = 0")
                .bind(hash)
                .fetch_optional(&self.pool)
                .await
                .map_err(db)?
                .map(KeyFila::a_dominio)
                .transpose()?
        else {
            return Ok(None);
        };

        let org = sqlx::query_as::<_, OrgFila>("SELECT * FROM organizaciones WHERE id = ?")
            .bind(clave.organizacion.to_string())
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?
            .map(OrgFila::a_dominio)
            .transpose()?;

        Ok(org.map(|org| (org, clave)))
    }

    async fn marcar_uso_api_key(
        &self,
        id: IdApiKey,
        ahora: DateTime<Utc>,
    ) -> Result<(), ErrorRepositorio> {
        sqlx::query("UPDATE api_keys SET ultimo_uso = ? WHERE id = ?")
            .bind(ahora.to_rfc3339())
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(())
    }
}

#[async_trait]
impl LibroCreditos for RepositorioSqlite {
    async fn saldo(&self, organizacion: IdOrganizacion) -> Result<u64, ErrorRepositorio> {
        let fila = sqlx::query("SELECT disponible FROM saldos WHERE organizacion = ?")
            .bind(organizacion.to_string())
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?;
        Ok(fila
            .map(|f| f.get::<i64, _>("disponible") as u64)
            .unwrap_or(0))
    }

    async fn movimientos(
        &self,
        organizacion: IdOrganizacion,
        limite: usize,
    ) -> Result<Vec<Movimiento>, ErrorRepositorio> {
        let filas = sqlx::query_as::<_, MovimientoFila>(
            "SELECT organizacion, tipo, cantidad, job_id, descripcion, creado_en
             FROM movimientos WHERE organizacion = ? ORDER BY id DESC LIMIT ?",
        )
        .bind(organizacion.to_string())
        .bind(limite as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        filas.into_iter().map(MovimientoFila::a_dominio).collect()
    }

    async fn acreditar(
        &self,
        organizacion: IdOrganizacion,
        cantidad: u64,
        descripcion: &str,
        ahora: DateTime<Utc>,
    ) -> Result<u64, ErrorRepositorio> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        sqlx::query(
            "INSERT INTO saldos (organizacion, disponible) VALUES (?, ?)
             ON CONFLICT(organizacion) DO UPDATE SET disponible = disponible + excluded.disponible",
        )
        .bind(organizacion.to_string())
        .bind(cantidad as i64)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        registrar_movimiento(
            &mut tx,
            organizacion,
            TipoMovimiento::Acreditacion,
            cantidad,
            None,
            descripcion,
            ahora,
        )
        .await?;
        let fila = sqlx::query("SELECT disponible FROM saldos WHERE organizacion = ?")
            .bind(organizacion.to_string())
            .fetch_one(&mut *tx)
            .await
            .map_err(db)?;
        tx.commit().await.map_err(db)?;
        Ok(fila.get::<i64, _>("disponible") as u64)
    }

    async fn reservar(
        &self,
        organizacion: IdOrganizacion,
        job: IdJob,
        cantidad: u64,
        ahora: DateTime<Utc>,
    ) -> Result<(), ErrorRepositorio> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        let disponible = sqlx::query("SELECT disponible FROM saldos WHERE organizacion = ?")
            .bind(organizacion.to_string())
            .fetch_optional(&mut *tx)
            .await
            .map_err(db)?
            .map(|f| f.get::<i64, _>("disponible") as u64)
            .unwrap_or(0);

        if disponible < cantidad {
            return Err(ErrorRepositorio::SaldoInsuficiente {
                disponible,
                pedido: cantidad,
            });
        }

        sqlx::query("UPDATE saldos SET disponible = disponible - ? WHERE organizacion = ?")
            .bind(cantidad as i64)
            .bind(organizacion.to_string())
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        sqlx::query(
            "INSERT INTO reservas_credito (job_id, organizacion, cantidad) VALUES (?, ?, ?)",
        )
        .bind(job.to_string())
        .bind(organizacion.to_string())
        .bind(cantidad as i64)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        registrar_movimiento(
            &mut tx,
            organizacion,
            TipoMovimiento::Reserva,
            cantidad,
            Some(job),
            &format!("reserva para {job}"),
            ahora,
        )
        .await?;
        tx.commit().await.map_err(db)?;
        Ok(())
    }

    async fn confirmar(
        &self,
        organizacion: IdOrganizacion,
        job: IdJob,
        cobrado: u64,
        ahora: DateTime<Utc>,
    ) -> Result<(), ErrorRepositorio> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        let Some(reservado) = sqlx::query("SELECT cantidad FROM reservas_credito WHERE job_id = ?")
            .bind(job.to_string())
            .fetch_optional(&mut *tx)
            .await
            .map_err(db)?
            .map(|f| f.get::<i64, _>("cantidad") as u64)
        else {
            return Ok(());
        };

        let devolver = reservado.saturating_sub(cobrado);
        if devolver > 0 {
            sqlx::query("UPDATE saldos SET disponible = disponible + ? WHERE organizacion = ?")
                .bind(devolver as i64)
                .bind(organizacion.to_string())
                .execute(&mut *tx)
                .await
                .map_err(db)?;
        }
        sqlx::query("DELETE FROM reservas_credito WHERE job_id = ?")
            .bind(job.to_string())
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        registrar_movimiento(
            &mut tx,
            organizacion,
            TipoMovimiento::Cobro,
            cobrado.min(reservado),
            Some(job),
            &format!("cobro de {job}"),
            ahora,
        )
        .await?;
        tx.commit().await.map_err(db)?;
        Ok(())
    }

    async fn liberar(
        &self,
        organizacion: IdOrganizacion,
        job: IdJob,
        ahora: DateTime<Utc>,
    ) -> Result<(), ErrorRepositorio> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        let Some(reservado) = sqlx::query("SELECT cantidad FROM reservas_credito WHERE job_id = ?")
            .bind(job.to_string())
            .fetch_optional(&mut *tx)
            .await
            .map_err(db)?
            .map(|f| f.get::<i64, _>("cantidad") as u64)
        else {
            return Ok(());
        };

        sqlx::query("UPDATE saldos SET disponible = disponible + ? WHERE organizacion = ?")
            .bind(reservado as i64)
            .bind(organizacion.to_string())
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        sqlx::query("DELETE FROM reservas_credito WHERE job_id = ?")
            .bind(job.to_string())
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        registrar_movimiento(
            &mut tx,
            organizacion,
            TipoMovimiento::Liberacion,
            reservado,
            Some(job),
            &format!("liberacion de {job}"),
            ahora,
        )
        .await?;
        tx.commit().await.map_err(db)?;
        Ok(())
    }
}

async fn registrar_movimiento(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    organizacion: IdOrganizacion,
    tipo: TipoMovimiento,
    cantidad: u64,
    job: Option<IdJob>,
    descripcion: &str,
    ahora: DateTime<Utc>,
) -> Result<(), ErrorRepositorio> {
    sqlx::query(
        "INSERT INTO movimientos (organizacion, tipo, cantidad, job_id, descripcion, creado_en)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(organizacion.to_string())
    .bind(serde_json::to_string(&tipo).map_err(|e| ErrorRepositorio::Interno(e.to_string()))?)
    .bind(cantidad as i64)
    .bind(job.map(|id| id.to_string()))
    .bind(descripcion)
    .bind(ahora.to_rfc3339())
    .execute(&mut **tx)
    .await
    .map_err(db)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dp_dominio::{Archivo, Job, OpcionesJob, Plan, RepositorioArchivos, TipoArchivo};

    async fn repo_tmp(etiqueta: &str) -> (RepositorioSqlite, std::path::PathBuf) {
        let raiz = std::env::temp_dir().join(format!(
            "dp-sqlite-{etiqueta}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&raiz).unwrap();
        let ruta = raiz.join("plataforma.sqlite");
        let repo = RepositorioSqlite::abrir(&ruta).await.unwrap();
        (repo, raiz)
    }

    fn job_de(org: IdOrganizacion, ahora: DateTime<Utc>) -> Job {
        Job::nuevo(
            org,
            "csv.inspect",
            IdArchivo::nuevo(),
            OpcionesJob::default(),
            Plan::Pro.limites(),
            1,
            ahora,
        )
    }

    #[tokio::test]
    async fn persiste_org_creditos_y_claves() {
        let (repo, raiz) = repo_tmp("cuentas").await;
        let ahora = Utc::now();
        let org = Organizacion::nueva("acme", Plan::Starter, ahora);
        let id = org.id;
        repo.crear_organizacion(org).await.unwrap();
        assert!(repo.alguna_organizacion().await.unwrap().is_some());
        assert_eq!(repo.acreditar(id, 20, "carga", ahora).await.unwrap(), 20);

        let job = IdJob::nuevo();
        repo.reservar(id, job, 5, ahora).await.unwrap();
        assert_eq!(repo.saldo(id).await.unwrap(), 15);
        repo.confirmar(id, job, 2, ahora).await.unwrap();
        assert_eq!(repo.saldo(id).await.unwrap(), 18);

        let (clave, token) = ApiKey::emitir(id, "cli", ahora);
        let hash = clave.hash.clone();
        repo.crear_api_key(clave).await.unwrap();
        assert!(repo.por_hash(&hash).await.unwrap().is_some());
        assert!(!token.is_empty());

        std::fs::remove_dir_all(raiz).ok();
    }

    #[tokio::test]
    async fn reclamar_no_entrega_el_mismo_job_dos_veces() {
        let (repo, raiz) = repo_tmp("cola").await;
        let ahora = Utc::now();
        let org = Organizacion::nueva("acme", Plan::Pro, ahora);
        let id = org.id;
        repo.crear_organizacion(org).await.unwrap();

        let a = job_de(id, ahora);
        let b = job_de(id, ahora + Duration::seconds(1));
        RepositorioJobs::crear(&repo, a).await.unwrap();
        RepositorioJobs::crear(&repo, b).await.unwrap();

        let plazo = Duration::seconds(60);
        let uno = repo
            .reclamar_siguiente("w1", plazo, ahora)
            .await
            .unwrap()
            .unwrap();
        let dos = repo
            .reclamar_siguiente("w2", plazo, ahora)
            .await
            .unwrap()
            .unwrap();
        assert_ne!(uno.id, dos.id);
        assert!(
            repo.reclamar_siguiente("w3", plazo, ahora)
                .await
                .unwrap()
                .is_none()
        );

        std::fs::remove_dir_all(raiz).ok();
    }

    #[tokio::test]
    async fn un_archivo_sobrevive_el_ciclo_crear_obtener() {
        let (repo, raiz) = repo_tmp("archivos").await;
        let ahora = Utc::now();
        let org = IdOrganizacion::nuevo();
        let mut archivo = Archivo::pendiente(org, "a.csv", ahora);
        archivo.confirmar(4, TipoArchivo::Csv, "abcd".to_string());
        let id = archivo.id;
        RepositorioArchivos::crear(&repo, archivo).await.unwrap();
        let recuperado = RepositorioArchivos::obtener(&repo, org, id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(recuperado.nombre_original, "a.csv");
        assert_eq!(recuperado.bytes, 4);
        std::fs::remove_dir_all(raiz).ok();
    }

    #[tokio::test]
    async fn sobrevive_cerrar_y_reabrir_el_archivo() {
        let raiz = std::env::temp_dir().join(format!(
            "dp-sqlite-reabrir-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&raiz).unwrap();
        let ruta = raiz.join("plataforma.sqlite");
        let ahora = Utc::now();
        let org = Organizacion::nueva("acme", Plan::Pro, ahora);
        let id = org.id;
        {
            let repo = RepositorioSqlite::abrir(&ruta).await.unwrap();
            repo.crear_organizacion(org).await.unwrap();
            repo.acreditar(id, 7, "carga", ahora).await.unwrap();
        }
        let repo = RepositorioSqlite::abrir(&ruta).await.unwrap();
        let recuperada = repo.obtener_organizacion(id).await.unwrap().unwrap();
        assert_eq!(recuperada.nombre, "acme");
        assert_eq!(repo.saldo(id).await.unwrap(), 7);
        std::fs::remove_dir_all(raiz).ok();
    }
}
