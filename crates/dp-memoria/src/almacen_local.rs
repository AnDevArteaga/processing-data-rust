//! Almacén sobre el sistema de archivos.

use async_trait::async_trait;
use dp_dominio::{Almacen, ErrorAlmacen};
use std::path::{Path, PathBuf};

pub struct AlmacenLocal {
    raiz: PathBuf,
}

impl AlmacenLocal {
    /// Crea el directorio raíz si no existe.
    pub async fn nuevo(raiz: impl Into<PathBuf>) -> Result<Self, ErrorAlmacen> {
        let raiz = raiz.into();
        tokio::fs::create_dir_all(&raiz).await?;
        Ok(AlmacenLocal { raiz })
    }

    /// Traduce una clave del almacén a una ruta del disco, **validándola**.
    ///
    /// Nosotros construimos las claves a partir de UUID, así que en teoría
    /// nunca contienen nada raro. Esta función existe porque «en teoría» no es
    /// una garantía de seguridad: es la última línea de defensa contra escribir
    /// fuera del directorio del almacén, y cuesta cuatro comprobaciones.
    fn ruta_de(&self, clave: &str) -> Result<PathBuf, ErrorAlmacen> {
        let invalida = clave.is_empty()
            || clave.contains("..")
            || clave.starts_with('/')
            || clave.starts_with('\\')
            // En Windows, "C:" en medio de una ruta relativa la vuelve absoluta.
            || clave.contains(':')
            || clave.chars().any(|c| c.is_control());

        if invalida {
            return Err(ErrorAlmacen::ClaveInvalida(clave.to_string()));
        }

        Ok(self.raiz.join(clave))
    }

    async fn asegurar_directorio(ruta: &Path) -> Result<(), ErrorAlmacen> {
        if let Some(padre) = ruta.parent() {
            tokio::fs::create_dir_all(padre).await?;
        }
        Ok(())
    }
}

#[async_trait]
impl Almacen for AlmacenLocal {
    async fn guardar(&self, clave: &str, bytes: &[u8]) -> Result<(), ErrorAlmacen> {
        let ruta = self.ruta_de(clave)?;
        Self::asegurar_directorio(&ruta).await?;
        tokio::fs::write(&ruta, bytes).await?;
        Ok(())
    }

    async fn leer(&self, clave: &str) -> Result<Vec<u8>, ErrorAlmacen> {
        let ruta = self.ruta_de(clave)?;
        tokio::fs::read(&ruta).await.map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                ErrorAlmacen::NoExiste(clave.to_string())
            } else {
                ErrorAlmacen::Io(error)
            }
        })
    }

    /// Sobre el sistema de archivos no hay nada que materializar: el objeto ya
    /// *es* un archivo local. Devolvemos su ruta sin copiar un byte.
    ///
    /// Esta es la implementación que hace que el motor pueda procesar 150 MB
    /// con 4 MB de memoria: recibe una ruta y hace streaming sobre ella.
    async fn materializar(&self, clave: &str) -> Result<PathBuf, ErrorAlmacen> {
        let ruta = self.ruta_de(clave)?;
        if !tokio::fs::try_exists(&ruta).await? {
            return Err(ErrorAlmacen::NoExiste(clave.to_string()));
        }
        Ok(ruta)
    }

    async fn subir(&self, clave: &str, origen: &Path) -> Result<(), ErrorAlmacen> {
        let destino = self.ruta_de(clave)?;
        Self::asegurar_directorio(&destino).await?;

        // Si el worker escribió directamente en su destino final, copiar sería
        // un error (mismo archivo de origen y destino).
        if destino == origen {
            return Ok(());
        }
        tokio::fs::copy(origen, &destino).await?;
        Ok(())
    }

    async fn existe(&self, clave: &str) -> Result<bool, ErrorAlmacen> {
        let ruta = self.ruta_de(clave)?;
        Ok(tokio::fs::try_exists(&ruta).await?)
    }

    async fn borrar(&self, clave: &str) -> Result<(), ErrorAlmacen> {
        let ruta = self.ruta_de(clave)?;
        match tokio::fs::remove_file(&ruta).await {
            Ok(()) => Ok(()),
            // Borrar algo que ya no está es un éxito, no un error: hace que la
            // tarea de limpieza se pueda repetir sin miedo.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(ErrorAlmacen::Io(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cada test usa su propio directorio temporal para poder correr en
    /// paralelo sin pisarse.
    async fn almacen_temporal(etiqueta: &str) -> (AlmacenLocal, PathBuf) {
        let raiz = std::env::temp_dir().join(format!(
            "dp-test-{etiqueta}-{}",
            uuid_simple_de_prueba(etiqueta)
        ));
        let almacen = AlmacenLocal::nuevo(&raiz).await.unwrap();
        (almacen, raiz)
    }

    /// Un identificador estable por etiqueta, para no depender de uuid en las
    /// dependencias de test.
    fn uuid_simple_de_prueba(etiqueta: &str) -> u64 {
        use std::hash::{DefaultHasher, Hash, Hasher};
        let mut h = DefaultHasher::new();
        etiqueta.hash(&mut h);
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .hash(&mut h);
        h.finish()
    }

    #[tokio::test]
    async fn guardar_y_leer_devuelve_los_mismos_bytes() {
        let (almacen, raiz) = almacen_temporal("ida-vuelta").await;

        almacen
            .guardar("objetos/abc", b"nombre,email\nJuan,j@x.com")
            .await
            .unwrap();

        let leido = almacen.leer("objetos/abc").await.unwrap();
        assert_eq!(leido, b"nombre,email\nJuan,j@x.com");
        assert!(almacen.existe("objetos/abc").await.unwrap());

        tokio::fs::remove_dir_all(raiz).await.ok();
    }

    #[tokio::test]
    async fn leer_algo_que_no_existe_da_no_existe_y_no_un_error_de_io() {
        let (almacen, raiz) = almacen_temporal("no-existe").await;

        let error = almacen.leer("objetos/fantasma").await.unwrap_err();
        assert!(matches!(error, ErrorAlmacen::NoExiste(_)));
        assert!(!almacen.existe("objetos/fantasma").await.unwrap());

        tokio::fs::remove_dir_all(raiz).await.ok();
    }

    /// El test de seguridad: ninguna clave puede escribir fuera de la raíz.
    #[tokio::test]
    async fn una_clave_con_escape_de_directorio_se_rechaza() {
        let (almacen, raiz) = almacen_temporal("traversal").await;

        for clave in [
            "../fuera.txt",
            "objetos/../../fuera.txt",
            "/etc/passwd",
            "\\Windows\\System32\\algo",
            "C:/Windows/algo",
            "objetos/con\0nulo",
            "",
        ] {
            let error = almacen
                .guardar(clave, b"x")
                .await
                .expect_err(&format!("la clave '{clave}' deberia rechazarse"));
            assert!(
                matches!(error, ErrorAlmacen::ClaveInvalida(_)),
                "la clave '{clave}' dio {error:?} en vez de ClaveInvalida"
            );
        }

        tokio::fs::remove_dir_all(raiz).await.ok();
    }

    /// Sobre disco, materializar no copia: devuelve la ruta real. Es lo que
    /// permite el streaming del motor.
    #[tokio::test]
    async fn materializar_devuelve_la_ruta_sin_copiar() {
        let (almacen, raiz) = almacen_temporal("materializar").await;
        almacen.guardar("objetos/xyz", b"datos").await.unwrap();

        let ruta = almacen.materializar("objetos/xyz").await.unwrap();
        assert!(ruta.starts_with(&raiz));
        assert_eq!(tokio::fs::read(&ruta).await.unwrap(), b"datos");

        tokio::fs::remove_dir_all(raiz).await.ok();
    }

    #[tokio::test]
    async fn materializar_algo_inexistente_falla() {
        let (almacen, raiz) = almacen_temporal("materializar-vacio").await;
        let error = almacen.materializar("objetos/nada").await.unwrap_err();
        assert!(matches!(error, ErrorAlmacen::NoExiste(_)));
        tokio::fs::remove_dir_all(raiz).await.ok();
    }

    /// Borrar dos veces no debe fallar: la limpieza periódica tiene que poder
    /// reintentarse sin condiciones de carrera.
    #[tokio::test]
    async fn borrar_es_idempotente() {
        let (almacen, raiz) = almacen_temporal("borrar").await;
        almacen.guardar("objetos/tmp", b"x").await.unwrap();

        almacen.borrar("objetos/tmp").await.unwrap();
        almacen.borrar("objetos/tmp").await.unwrap();
        assert!(!almacen.existe("objetos/tmp").await.unwrap());

        tokio::fs::remove_dir_all(raiz).await.ok();
    }

    #[tokio::test]
    async fn subir_un_archivo_local_lo_copia_al_almacen() {
        let (almacen, raiz) = almacen_temporal("subir").await;

        let temporal =
            std::env::temp_dir().join(format!("dp-origen-{}.csv", uuid_simple_de_prueba("origen")));
        tokio::fs::write(&temporal, b"salida,limpia").await.unwrap();

        almacen.subir("objetos/salida", &temporal).await.unwrap();
        assert_eq!(
            almacen.leer("objetos/salida").await.unwrap(),
            b"salida,limpia"
        );

        tokio::fs::remove_file(temporal).await.ok();
        tokio::fs::remove_dir_all(raiz).await.ok();
    }
}
