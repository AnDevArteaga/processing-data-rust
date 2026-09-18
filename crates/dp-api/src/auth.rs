//! Autenticación.
//!
//! Un extractor de Axum produce una `Identidad`. Todo manejador de ruta la
//! pide como argumento: si un endpoint olvida pedirla, no tiene de dónde sacar
//! la organización, así que no compila.

use crate::error::ErrorApi;
use crate::estado::Estado;
use async_trait::async_trait;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use dp_dominio::{IdOrganizacion, Plan, RepositorioCuentas, huella_de_token};
use std::sync::Arc;

/// Quién está haciendo la petición y con qué plan.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Identidad {
    pub organizacion: IdOrganizacion,
    pub plan: Plan,
}

#[async_trait]
pub trait Autenticador: Send + Sync {
    /// `None` significa credencial desconocida. No distinguimos "no existe" de
    /// "revocada" a propósito: no hay que ayudar a enumerar credenciales.
    async fn resolver(&self, token: &str) -> Option<Identidad>;
}

/// Un único token válido. Sirve para tests y para `DP_MEMORIA=1`.
pub struct AutenticadorFijo {
    token: String,
    identidad: Identidad,
}

impl AutenticadorFijo {
    pub fn nuevo(token: impl Into<String>, organizacion: IdOrganizacion, plan: Plan) -> Self {
        AutenticadorFijo {
            token: token.into(),
            identidad: Identidad { organizacion, plan },
        }
    }

    pub fn identidad(&self) -> Identidad {
        self.identidad
    }
}

#[async_trait]
impl Autenticador for AutenticadorFijo {
    async fn resolver(&self, token: &str) -> Option<Identidad> {
        // Comparación de tiempo constante, igual que con las firmas: el token
        // es una credencial y compararlo con `==` filtra información por
        // tiempo. `ct_eq` no existe en la librería estándar, así que sumamos
        // las diferencias byte a byte sin cortar el bucle.
        if iguales_en_tiempo_constante(self.token.as_bytes(), token.as_bytes()) {
            Some(self.identidad)
        } else {
            None
        }
    }
}

/// Comparación que tarda lo mismo acierte o no.
///
/// El detalle de longitud sí se filtra (y no importa: la longitud del token es
/// pública), pero el contenido no.
fn iguales_en_tiempo_constante(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    // `fold` recorre TODOS los bytes siempre. Un `all` o un `==` cortarían en
    // la primera diferencia, y esa es exactamente la fuga.
    a.iter()
        .zip(b)
        .fold(0u8, |acumulado, (x, y)| acumulado | (x ^ y))
        == 0
}

/// El extractor. Lee `Authorization: Bearer <token>` y lo resuelve.
impl FromRequestParts<Arc<Estado>> for Identidad {
    type Rejection = ErrorApi;

    async fn from_request_parts(
        parts: &mut Parts,
        estado: &Arc<Estado>,
    ) -> Result<Self, Self::Rejection> {
        let token = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|valor| valor.to_str().ok())
            .and_then(|valor| valor.strip_prefix("Bearer "))
            .map(str::trim)
            .ok_or(ErrorApi::NoAutorizado)?;

        estado
            .autenticador
            .resolver(token)
            .await
            .ok_or(ErrorApi::NoAutorizado)
    }
}

/// Resuelve tokens contra el repositorio de cuentas.
pub struct AutenticadorCuentas {
    cuentas: Arc<dyn RepositorioCuentas>,
}

impl AutenticadorCuentas {
    pub fn nuevo(cuentas: Arc<dyn RepositorioCuentas>) -> Self {
        AutenticadorCuentas { cuentas }
    }
}

#[async_trait]
impl Autenticador for AutenticadorCuentas {
    async fn resolver(&self, token: &str) -> Option<Identidad> {
        let (org, clave) = self
            .cuentas
            .por_hash(&huella_de_token(token))
            .await
            .ok()??;
        let _ = self
            .cuentas
            .marcar_uso_api_key(clave.id, chrono::Utc::now())
            .await;
        Some(Identidad {
            organizacion: org.id,
            plan: org.plan,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn autenticador() -> AutenticadorFijo {
        AutenticadorFijo::nuevo("dp_dev_secreto", IdOrganizacion::nuevo(), Plan::Pro)
    }

    #[tokio::test]
    async fn el_token_correcto_resuelve_la_identidad() {
        let a = autenticador();
        let identidad = a
            .resolver("dp_dev_secreto")
            .await
            .expect("deberia resolver");
        assert_eq!(identidad.plan, Plan::Pro);
        assert_eq!(identidad.organizacion, a.identidad().organizacion);
    }

    #[tokio::test]
    async fn un_token_equivocado_no_resuelve() {
        assert!(autenticador().resolver("dp_dev_otro").await.is_none());
        assert!(autenticador().resolver("").await.is_none());
        assert!(autenticador().resolver("dp_dev_secret").await.is_none());
    }

    #[test]
    fn la_comparacion_en_tiempo_constante_es_correcta() {
        assert!(iguales_en_tiempo_constante(b"abc", b"abc"));
        assert!(!iguales_en_tiempo_constante(b"abc", b"abd"));
        assert!(!iguales_en_tiempo_constante(b"abc", b"ab"));
        assert!(iguales_en_tiempo_constante(b"", b""));
    }
}
