//! Los puertos: todo lo que el dominio necesita del mundo exterior.
//!
//! Son traits y no structs porque es lo que permite que la API y el worker
//! corran hoy con implementaciones en memoria, y mañana contra PostgreSQL y S3
//! sin que cambie una línea de la lógica.
//!
//! ## Por qué `#[async_trait]`
//!
//! `async fn` en traits ya es estable en Rust, pero un trait con `async fn`
//! **no se puede usar como `dyn Trait`**, y nosotros necesitamos exactamente
//! eso: la API guarda un `Arc<dyn RepositorioJobs>` para poder cambiar la
//! implementación en tiempo de ejecución y en los tests. El macro reescribe
//! cada `async fn` a una función que devuelve `Pin<Box<dyn Future + Send>>`,
//! que sí es compatible con `dyn`. Cuesta una asignación por llamada, y es
//! irrelevante frente a una consulta a base de datos.

use crate::archivo::Archivo;
use crate::error::{ErrorAlmacen, ErrorRepositorio};
use crate::ids::{IdArchivo, IdJob, IdOrganizacion};
use crate::job::{EstadoJob, Job};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use std::path::{Path, PathBuf};

/// Dónde viven los bytes.
///
/// `Send + Sync` porque el mismo almacén se comparte entre todas las tareas
/// del servidor. Sin esos límites, `Arc<dyn Almacen>` no se podría mover entre
/// hilos y `tokio` lo rechazaría al compilar.
#[async_trait]
pub trait Almacen: Send + Sync {
    async fn guardar(&self, clave: &str, bytes: &[u8]) -> Result<(), ErrorAlmacen>;

    async fn leer(&self, clave: &str) -> Result<Vec<u8>, ErrorAlmacen>;

    /// Deja el objeto disponible como archivo local y devuelve su ruta.
    ///
    /// Existe porque el motor procesa en streaming **desde disco**: recibe una
    /// ruta, no un buffer, y eso es justo lo que le permite tratar un archivo
    /// de 150 MB con 4 MB de memoria. Sobre el sistema de archivos esto no
    /// copia nada; contra S3 descargará a un temporal.
    async fn materializar(&self, clave: &str) -> Result<PathBuf, ErrorAlmacen>;

    /// Sube un archivo local ya escrito. La contraparte de `materializar`.
    async fn subir(&self, clave: &str, ruta: &Path) -> Result<(), ErrorAlmacen>;

    async fn existe(&self, clave: &str) -> Result<bool, ErrorAlmacen>;

    async fn borrar(&self, clave: &str) -> Result<(), ErrorAlmacen>;
}

#[async_trait]
pub trait RepositorioArchivos: Send + Sync {
    async fn crear(&self, archivo: Archivo) -> Result<(), ErrorRepositorio>;

    /// La organización va en la firma, no como filtro posterior.
    ///
    /// Es la defensa contra la fuga de datos entre clientes de la que habla la
    /// sección G de la arquitectura: si el identificador de la organización es
    /// obligatorio para *obtener* un archivo, es imposible escribir un
    /// endpoint que se olvide de comprobar el dueño.
    async fn obtener(
        &self,
        organizacion: IdOrganizacion,
        id: IdArchivo,
    ) -> Result<Option<Archivo>, ErrorRepositorio>;

    async fn actualizar(&self, archivo: Archivo) -> Result<(), ErrorRepositorio>;

    /// Los que ya pasaron su fecha de retención, para borrarlos.
    async fn vencidos(&self, ahora: DateTime<Utc>) -> Result<Vec<Archivo>, ErrorRepositorio>;
}

/// El repositorio de jobs **es** la cola.
///
/// Esto es la decisión ADR-2 hecha código: `reclamar_siguiente` es el `SELECT
/// ... FOR UPDATE SKIP LOCKED` de PostgreSQL. Tener la cola y el estado en el
/// mismo sitio hace que tomar un mensaje y cambiar el estado ocurran en una
/// sola transacción, y eso elimina la clase entera de errores en que un job se
/// entrega dos veces o se pierde.
#[async_trait]
pub trait RepositorioJobs: Send + Sync {
    async fn crear(&self, job: Job) -> Result<(), ErrorRepositorio>;

    async fn obtener(
        &self,
        organizacion: IdOrganizacion,
        id: IdJob,
    ) -> Result<Option<Job>, ErrorRepositorio>;

    async fn listar(
        &self,
        organizacion: IdOrganizacion,
        estado: Option<EstadoJob>,
        limite: usize,
    ) -> Result<Vec<Job>, ErrorRepositorio>;

    /// Cuántos jobs sin terminar tiene una organización. Es lo que hace
    /// cumplir el límite `jobs_en_vuelo` del plan.
    async fn en_vuelo(&self, organizacion: IdOrganizacion) -> Result<usize, ErrorRepositorio>;

    /// Toma el siguiente job encolado de forma atómica.
    ///
    /// Devuelve `None` si no hay nada. Es responsabilidad de la
    /// implementación garantizar que dos workers concurrentes nunca reciban el
    /// mismo job.
    async fn reclamar_siguiente(
        &self,
        worker: &str,
        plazo: Duration,
        ahora: DateTime<Utc>,
    ) -> Result<Option<Job>, ErrorRepositorio>;

    /// Guarda un job que el worker ya modificó.
    ///
    /// Recibe el job completo en vez de campos sueltos porque las transiciones
    /// las decide el dominio: el worker llama a `job.completar(...)` y luego
    /// persiste el resultado. Así la máquina de estados vive en un solo lugar.
    async fn guardar(&self, job: Job) -> Result<(), ErrorRepositorio>;

    async fn cancelar(
        &self,
        organizacion: IdOrganizacion,
        id: IdJob,
        ahora: DateTime<Utc>,
    ) -> Result<Job, ErrorRepositorio>;

    /// Devuelve a la cola los jobs cuyo worker desapareció.
    /// Devuelve cuántos rescató.
    async fn rescatar_vencidos(&self, ahora: DateTime<Utc>) -> Result<u64, ErrorRepositorio>;
}

/// El reloj, como puerto.
///
/// Parece exagerado hasta que hay que probar «un job cuyo plazo venció hace
/// diez minutos». Con `Utc::now()` incrustado en la lógica, ese test tendría
/// que dormir de verdad. Con este trait se adelanta el reloj y el test tarda
/// microsegundos.
pub trait Reloj: Send + Sync {
    fn ahora(&self) -> DateTime<Utc>;
}

pub struct RelojDelSistema;

impl Reloj for RelojDelSistema {
    fn ahora(&self) -> DateTime<Utc> {
        Utc::now()
    }
}
