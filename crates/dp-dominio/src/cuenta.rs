//! Organización y credenciales de acceso.

use crate::ids::{IdApiKey, IdOrganizacion};
use crate::plan::Plan;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Huella que se guarda. El token en claro solo se muestra una vez, al crearlo.
pub fn huella_de_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Organizacion {
    pub id: IdOrganizacion,
    pub nombre: String,
    pub plan: Plan,
    pub creado_en: DateTime<Utc>,
}

impl Organizacion {
    pub fn nueva(nombre: impl Into<String>, plan: Plan, ahora: DateTime<Utc>) -> Self {
        Organizacion {
            id: IdOrganizacion::nuevo(),
            nombre: nombre.into(),
            plan,
            creado_en: ahora,
        }
    }
}

/// Metadatos de una API key. El secreto no vive aquí.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApiKey {
    pub id: IdApiKey,
    pub organizacion: IdOrganizacion,
    pub nombre: String,
    /// Primeros caracteres del token, para que el cliente reconozca la clave
    /// en el listado sin que guardemos el secreto.
    pub prefijo: String,
    pub hash: String,
    pub revocada: bool,
    pub ultimo_uso: Option<DateTime<Utc>>,
    pub creada_en: DateTime<Utc>,
}

impl ApiKey {
    /// Crea la clave y devuelve el token en claro. A partir de aquí solo
    /// existe el hash: si se pierde el token, hay que emitir otro.
    pub fn emitir(
        organizacion: IdOrganizacion,
        nombre: impl Into<String>,
        ahora: DateTime<Utc>,
    ) -> (Self, String) {
        let secreto = uuid::Uuid::new_v4().simple().to_string();
        let token = format!("dp_{secreto}");
        let prefijo = token.chars().take(12).collect();
        let key = ApiKey {
            id: IdApiKey::nuevo(),
            organizacion,
            nombre: nombre.into(),
            prefijo,
            hash: huella_de_token(&token),
            revocada: false,
            ultimo_uso: None,
            creada_en: ahora,
        };
        (key, token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_token_no_se_puede_recuperar_del_hash() {
        let (key, token) = ApiKey::emitir(IdOrganizacion::nuevo(), "prod", Utc::now());
        assert!(token.starts_with("dp_"));
        assert_eq!(key.hash, huella_de_token(&token));
        assert_ne!(key.hash, token);
        assert!(key.prefijo.starts_with("dp_"));
        assert_eq!(key.prefijo.len(), 12);
    }

    #[test]
    fn dos_emisiones_producen_tokens_distintos() {
        let org = IdOrganizacion::nuevo();
        let ahora = Utc::now();
        let (a, ta) = ApiKey::emitir(org, "a", ahora);
        let (b, tb) = ApiKey::emitir(org, "b", ahora);
        assert_ne!(ta, tb);
        assert_ne!(a.hash, b.hash);
    }
}
