//! Repositorio en memoria.
//!
//! Implementa los dos repositorios y la cola. Es fiel al comportamiento que
//! tendrá PostgreSQL en lo que importa: el filtro por organización es
//! obligatorio, `reclamar_siguiente` es atómico, y el rescate por plazo
//! vencido existe.

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use dp_dominio::{
    Archivo, ErrorRepositorio, EstadoJob, IdArchivo, IdJob, IdOrganizacion, Job,
    RepositorioArchivos, RepositorioJobs,
};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Default)]
struct Datos {
    archivos: HashMap<IdArchivo, Archivo>,
    jobs: HashMap<IdJob, Job>,
}

#[derive(Default)]
pub struct RepositorioEnMemoria {
    // `std::sync::Mutex` y no el de tokio: las secciones críticas son
    // asignaciones a un HashMap, sin ningún `await` dentro. El mutex de tokio
    // solo hace falta cuando hay que mantener el candado a través de una
    // espera, y ahí es donde nace la mitad de los bloqueos mutuos.
    datos: Mutex<Datos>,
}

impl RepositorioEnMemoria {
    pub fn nuevo() -> Self {
        Self::default()
    }

    fn con_datos<T>(&self, f: impl FnOnce(&mut Datos) -> T) -> T {
        let mut guardia = self
            .datos
            .lock()
            .expect("el mutex del repositorio no se envenena");
        f(&mut guardia)
    }
}

#[async_trait]
impl RepositorioArchivos for RepositorioEnMemoria {
    async fn crear(&self, archivo: Archivo) -> Result<(), ErrorRepositorio> {
        self.con_datos(|datos| {
            datos.archivos.insert(archivo.id, archivo);
        });
        Ok(())
    }

    async fn obtener(
        &self,
        organizacion: IdOrganizacion,
        id: IdArchivo,
    ) -> Result<Option<Archivo>, ErrorRepositorio> {
        Ok(self.con_datos(|datos| {
            datos
                .archivos
                .get(&id)
                // El filtro por organización va en la MISMA búsqueda. Un
                // archivo de otro cliente se comporta como si no existiera,
                // que además no le revela al atacante si el id es válido.
                .filter(|archivo| archivo.organizacion == organizacion)
                .cloned()
        }))
    }

    async fn actualizar(&self, archivo: Archivo) -> Result<(), ErrorRepositorio> {
        self.con_datos(|datos| {
            if !datos.archivos.contains_key(&archivo.id) {
                return Err(ErrorRepositorio::no_encontrado("archivo", archivo.id));
            }
            datos.archivos.insert(archivo.id, archivo);
            Ok(())
        })
    }

    async fn vencidos(&self, ahora: DateTime<Utc>) -> Result<Vec<Archivo>, ErrorRepositorio> {
        Ok(self.con_datos(|datos| {
            datos
                .archivos
                .values()
                .filter(|archivo| archivo.ha_vencido(ahora))
                .cloned()
                .collect()
        }))
    }
}

#[async_trait]
impl RepositorioJobs for RepositorioEnMemoria {
    async fn crear(&self, job: Job) -> Result<(), ErrorRepositorio> {
        self.con_datos(|datos| {
            datos.jobs.insert(job.id, job);
        });
        Ok(())
    }

    async fn obtener(
        &self,
        organizacion: IdOrganizacion,
        id: IdJob,
    ) -> Result<Option<Job>, ErrorRepositorio> {
        Ok(self.con_datos(|datos| {
            datos
                .jobs
                .get(&id)
                .filter(|job| job.organizacion == organizacion)
                .cloned()
        }))
    }

    async fn listar(
        &self,
        organizacion: IdOrganizacion,
        estado: Option<EstadoJob>,
        limite: usize,
    ) -> Result<Vec<Job>, ErrorRepositorio> {
        Ok(self.con_datos(|datos| {
            let mut encontrados: Vec<Job> = datos
                .jobs
                .values()
                .filter(|job| job.organizacion == organizacion)
                .filter(|job| estado.is_none_or(|buscado| job.estado == buscado))
                .cloned()
                .collect();

            // Más reciente primero, que es lo que espera un listado de jobs.
            encontrados.sort_by_key(|a| std::cmp::Reverse(a.creado_en));
            encontrados.truncate(limite);
            encontrados
        }))
    }

    async fn en_vuelo(&self, organizacion: IdOrganizacion) -> Result<usize, ErrorRepositorio> {
        Ok(self.con_datos(|datos| {
            datos
                .jobs
                .values()
                .filter(|job| job.organizacion == organizacion)
                .filter(|job| !job.estado.es_final())
                .count()
        }))
    }

    async fn reclamar_siguiente(
        &self,
        worker: &str,
        plazo: Duration,
        ahora: DateTime<Utc>,
    ) -> Result<Option<Job>, ErrorRepositorio> {
        // Todo esto pasa con el candado tomado, y ahí está la atomicidad: dos
        // workers concurrentes no pueden ver el mismo job en estado ENCOLADO.
        // En PostgreSQL el papel del candado lo hace `FOR UPDATE SKIP LOCKED`.
        self.con_datos(|datos| {
            let siguiente = datos
                .jobs
                .values()
                .filter(|job| job.estado == EstadoJob::Encolado)
                // El más antiguo primero: una cola justa, sin inanición.
                .min_by_key(|job| job.creado_en)
                .map(|job| job.id);

            let Some(id) = siguiente else {
                return Ok(None);
            };

            let job = datos
                .jobs
                .get_mut(&id)
                .expect("acabamos de encontrarlo con el candado tomado");

            job.reclamar(worker, ahora, plazo)?;
            Ok(Some(job.clone()))
        })
    }

    async fn guardar(&self, job: Job) -> Result<(), ErrorRepositorio> {
        self.con_datos(|datos| {
            if !datos.jobs.contains_key(&job.id) {
                return Err(ErrorRepositorio::no_encontrado("job", job.id));
            }
            datos.jobs.insert(job.id, job);
            Ok(())
        })
    }

    async fn cancelar(
        &self,
        organizacion: IdOrganizacion,
        id: IdJob,
        ahora: DateTime<Utc>,
    ) -> Result<Job, ErrorRepositorio> {
        self.con_datos(|datos| {
            let job = datos
                .jobs
                .get_mut(&id)
                .filter(|job| job.organizacion == organizacion)
                .ok_or_else(|| ErrorRepositorio::no_encontrado("job", id))?;

            // La transición la valida el dominio. Si el job ya está
            // procesando, esto devuelve TransicionInvalida y el `?` la
            // convierte en ErrorRepositorio.
            job.cancelar(ahora)?;
            Ok(job.clone())
        })
    }

    async fn rescatar_vencidos(&self, ahora: DateTime<Utc>) -> Result<u64, ErrorRepositorio> {
        Ok(self.con_datos(|datos| {
            let mut rescatados = 0;
            for job in datos.jobs.values_mut() {
                if job.rescatar(ahora).is_some() {
                    rescatados += 1;
                }
            }
            rescatados
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dp_core::Resumen;
    use dp_dominio::{ErrorDelJob, OpcionesJob, TipoArchivo};

    fn job_de(organizacion: IdOrganizacion, creado_en: DateTime<Utc>) -> Job {
        let mut job = Job::nuevo(
            organizacion,
            "csv.clean",
            IdArchivo::nuevo(),
            OpcionesJob::default(),
            dp_dominio::Plan::Pro.limites(),
            5,
            creado_en,
        );
        job.creado_en = creado_en;
        job
    }

    #[tokio::test]
    async fn un_job_se_guarda_y_se_recupera() {
        let repo = RepositorioEnMemoria::nuevo();
        let org = IdOrganizacion::nuevo();
        let job = job_de(org, Utc::now());
        let id = job.id;

        RepositorioJobs::crear(&repo, job).await.unwrap();
        let recuperado = RepositorioJobs::obtener(&repo, org, id).await.unwrap();

        assert_eq!(recuperado.unwrap().id, id);
    }

    /// El test de aislamiento entre clientes. Si esto se rompe, es una fuga de
    /// datos, no un bug de comodidad.
    #[tokio::test]
    async fn una_organizacion_no_puede_ver_el_job_de_otra() {
        let repo = RepositorioEnMemoria::nuevo();
        let mia = IdOrganizacion::nuevo();
        let ajena = IdOrganizacion::nuevo();

        let job = job_de(mia, Utc::now());
        let id = job.id;
        RepositorioJobs::crear(&repo, job).await.unwrap();

        // Con el identificador correcto pero la organización equivocada, el
        // job simplemente no existe.
        assert!(
            RepositorioJobs::obtener(&repo, ajena, id)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            RepositorioJobs::obtener(&repo, mia, id)
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn una_organizacion_no_puede_ver_el_archivo_de_otra() {
        let repo = RepositorioEnMemoria::nuevo();
        let mia = IdOrganizacion::nuevo();
        let ajena = IdOrganizacion::nuevo();

        let archivo = Archivo::pendiente(mia, "clientes.csv", Utc::now());
        let id = archivo.id;
        RepositorioArchivos::crear(&repo, archivo).await.unwrap();

        assert!(
            RepositorioArchivos::obtener(&repo, ajena, id)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            RepositorioArchivos::obtener(&repo, mia, id)
                .await
                .unwrap()
                .is_some()
        );
    }

    /// La garantía central de la cola: dos workers nunca reciben el mismo job.
    #[tokio::test]
    async fn dos_workers_reclaman_jobs_distintos() {
        let repo = RepositorioEnMemoria::nuevo();
        let org = IdOrganizacion::nuevo();
        let ahora = Utc::now();

        let primero = job_de(org, ahora);
        let segundo = job_de(org, ahora + Duration::seconds(1));
        RepositorioJobs::crear(&repo, primero).await.unwrap();
        RepositorioJobs::crear(&repo, segundo).await.unwrap();

        let plazo = Duration::seconds(60);
        let a = repo
            .reclamar_siguiente("worker-a", plazo, ahora)
            .await
            .unwrap()
            .expect("deberia haber un job");
        let b = repo
            .reclamar_siguiente("worker-b", plazo, ahora)
            .await
            .unwrap()
            .expect("deberia haber otro job");

        assert_ne!(a.id, b.id);
        // Y ya no queda nada por reclamar.
        assert!(
            repo.reclamar_siguiente("worker-c", plazo, ahora)
                .await
                .unwrap()
                .is_none()
        );
    }

    /// Cola justa: se atiende el más antiguo primero, así que un job no se
    /// queda esperando para siempre.
    #[tokio::test]
    async fn se_reclama_el_job_mas_antiguo_primero() {
        let repo = RepositorioEnMemoria::nuevo();
        let org = IdOrganizacion::nuevo();
        let ahora = Utc::now();

        let viejo = job_de(org, ahora - Duration::hours(1));
        let id_viejo = viejo.id;
        let nuevo = job_de(org, ahora);

        // Se insertan en orden inverso a propósito.
        RepositorioJobs::crear(&repo, nuevo).await.unwrap();
        RepositorioJobs::crear(&repo, viejo).await.unwrap();

        let reclamado = repo
            .reclamar_siguiente("worker-a", Duration::seconds(60), ahora)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(reclamado.id, id_viejo);
    }

    /// El escenario de resiliencia de punta a punta: un worker toma un job,
    /// muere, y otro worker lo recupera.
    #[tokio::test]
    async fn un_job_de_un_worker_muerto_vuelve_a_la_cola() {
        let repo = RepositorioEnMemoria::nuevo();
        let org = IdOrganizacion::nuevo();
        let inicio = Utc::now();

        RepositorioJobs::crear(&repo, job_de(org, inicio))
            .await
            .unwrap();

        let plazo = Duration::seconds(60);
        let reclamado = repo
            .reclamar_siguiente("worker-que-muere", plazo, inicio)
            .await
            .unwrap()
            .unwrap();

        // El worker desaparece: nadie llama a guardar. No hay nada que
        // reclamar mientras su plazo siga vigente.
        assert!(
            repo.reclamar_siguiente("worker-vivo", plazo, inicio)
                .await
                .unwrap()
                .is_none()
        );

        // Pasa el plazo y el barrido lo rescata.
        let despues = inicio + Duration::seconds(61);
        assert_eq!(repo.rescatar_vencidos(despues).await.unwrap(), 1);

        let recuperado = repo
            .reclamar_siguiente("worker-vivo", plazo, despues)
            .await
            .unwrap()
            .expect("otro worker deberia poder tomarlo");

        assert_eq!(recuperado.id, reclamado.id);
        // Es el segundo intento del mismo job, no un job nuevo.
        assert_eq!(recuperado.intentos, 2);
    }

    #[tokio::test]
    async fn el_barrido_no_toca_jobs_dentro_de_plazo() {
        let repo = RepositorioEnMemoria::nuevo();
        let org = IdOrganizacion::nuevo();
        let inicio = Utc::now();

        RepositorioJobs::crear(&repo, job_de(org, inicio))
            .await
            .unwrap();
        repo.reclamar_siguiente("worker-a", Duration::seconds(60), inicio)
            .await
            .unwrap();

        assert_eq!(
            repo.rescatar_vencidos(inicio + Duration::seconds(30))
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn cancelar_un_job_en_proceso_falla_con_transicion_invalida() {
        let repo = RepositorioEnMemoria::nuevo();
        let org = IdOrganizacion::nuevo();
        let ahora = Utc::now();

        let job = job_de(org, ahora);
        let id = job.id;
        RepositorioJobs::crear(&repo, job).await.unwrap();

        // Encolado: se puede cancelar.
        let cancelado = repo.cancelar(org, id, ahora).await.unwrap();
        assert_eq!(cancelado.estado, EstadoJob::Cancelado);

        // Y ya no se puede volver a cancelar.
        let error = repo.cancelar(org, id, ahora).await.unwrap_err();
        assert!(matches!(error, ErrorRepositorio::Transicion(_)));
    }

    #[tokio::test]
    async fn cancelar_el_job_de_otra_organizacion_no_lo_encuentra() {
        let repo = RepositorioEnMemoria::nuevo();
        let mia = IdOrganizacion::nuevo();
        let ajena = IdOrganizacion::nuevo();
        let ahora = Utc::now();

        let job = job_de(mia, ahora);
        let id = job.id;
        RepositorioJobs::crear(&repo, job).await.unwrap();

        let error = repo.cancelar(ajena, id, ahora).await.unwrap_err();
        assert!(matches!(error, ErrorRepositorio::NoEncontrado { .. }));
    }

    #[tokio::test]
    async fn los_jobs_en_vuelo_solo_cuentan_los_no_terminados() {
        let repo = RepositorioEnMemoria::nuevo();
        let org = IdOrganizacion::nuevo();
        let ahora = Utc::now();

        for _ in 0..3 {
            RepositorioJobs::crear(&repo, job_de(org, ahora))
                .await
                .unwrap();
        }
        assert_eq!(repo.en_vuelo(org).await.unwrap(), 3);

        // Terminamos uno.
        let mut job = repo
            .reclamar_siguiente("worker-a", Duration::seconds(60), ahora)
            .await
            .unwrap()
            .unwrap();
        job.completar(None, Resumen::default(), 5, ahora).unwrap();
        RepositorioJobs::guardar(&repo, job).await.unwrap();

        assert_eq!(repo.en_vuelo(org).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn listar_filtra_por_estado_y_ordena_por_recencia() {
        let repo = RepositorioEnMemoria::nuevo();
        let org = IdOrganizacion::nuevo();
        let ahora = Utc::now();

        RepositorioJobs::crear(&repo, job_de(org, ahora - Duration::hours(2)))
            .await
            .unwrap();
        RepositorioJobs::crear(&repo, job_de(org, ahora))
            .await
            .unwrap();

        let todos = repo.listar(org, None, 10).await.unwrap();
        assert_eq!(todos.len(), 2);
        assert!(todos[0].creado_en > todos[1].creado_en);

        // Fallamos uno y filtramos por estado.
        let mut job = repo
            .reclamar_siguiente("worker-a", Duration::seconds(60), ahora)
            .await
            .unwrap()
            .unwrap();
        job.fallar(ErrorDelJob::del_cliente("E_X", "malo"), ahora)
            .unwrap();
        RepositorioJobs::guardar(&repo, job).await.unwrap();

        let fallidos = repo
            .listar(org, Some(EstadoJob::Fallido), 10)
            .await
            .unwrap();
        assert_eq!(fallidos.len(), 1);
    }

    #[tokio::test]
    async fn el_limite_del_listado_se_respeta() {
        let repo = RepositorioEnMemoria::nuevo();
        let org = IdOrganizacion::nuevo();
        let ahora = Utc::now();

        for i in 0..5 {
            RepositorioJobs::crear(&repo, job_de(org, ahora + Duration::seconds(i)))
                .await
                .unwrap();
        }
        assert_eq!(repo.listar(org, None, 2).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn guardar_un_job_que_no_existe_falla() {
        let repo = RepositorioEnMemoria::nuevo();
        let job = job_de(IdOrganizacion::nuevo(), Utc::now());

        let error = RepositorioJobs::guardar(&repo, job).await.unwrap_err();
        assert!(matches!(error, ErrorRepositorio::NoEncontrado { .. }));
    }

    #[tokio::test]
    async fn los_archivos_vencidos_se_listan_para_borrarlos() {
        let repo = RepositorioEnMemoria::nuevo();
        let org = IdOrganizacion::nuevo();
        let ahora = Utc::now();

        let mut viejo = Archivo::pendiente(org, "viejo.csv", ahora - Duration::days(2));
        viejo.confirmar(10, TipoArchivo::Csv, "abc".to_string());
        RepositorioArchivos::crear(&repo, viejo).await.unwrap();
        RepositorioArchivos::crear(&repo, Archivo::pendiente(org, "nuevo.csv", ahora))
            .await
            .unwrap();

        let vencidos = RepositorioArchivos::vencidos(&repo, ahora).await.unwrap();
        assert_eq!(vencidos.len(), 1);
        assert_eq!(vencidos[0].nombre_original, "viejo.csv");
    }
}
