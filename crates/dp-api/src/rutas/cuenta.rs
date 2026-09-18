//! Cuenta, créditos y API keys.

use crate::auth::Identidad;
use crate::dto::{
    PeticionCrearClave, RespuestaClave, RespuestaClaveCreada, RespuestaCreditos, RespuestaCuenta,
    RespuestaLista,
};
use crate::error::ErrorApi;
use crate::estado::Estado;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use dp_dominio::{ApiKey, IdApiKey};
use std::sync::Arc;

pub async fn yo(
    State(estado): State<Arc<Estado>>,
    identidad: Identidad,
) -> Result<impl IntoResponse, ErrorApi> {
    let org = estado
        .cuentas
        .obtener_organizacion(identidad.organizacion)
        .await?
        .ok_or(ErrorApi::NoAutorizado)?;
    let credits = estado.libro.saldo(identidad.organizacion).await?;

    Ok(Json(RespuestaCuenta {
        organization_id: org.id,
        name: org.nombre,
        plan: org.plan,
        credits,
        limits: org.plan.limites(),
    }))
}

pub async fn creditos(
    State(estado): State<Arc<Estado>>,
    identidad: Identidad,
) -> Result<impl IntoResponse, ErrorApi> {
    let available = estado.libro.saldo(identidad.organizacion).await?;
    Ok(Json(RespuestaCreditos {
        available,
        monthly_allowance: identidad.plan.limites().creditos_mensuales,
    }))
}

pub async fn movimientos(
    State(estado): State<Arc<Estado>>,
    identidad: Identidad,
) -> Result<impl IntoResponse, ErrorApi> {
    let data = estado.libro.movimientos(identidad.organizacion, 50).await?;
    Ok(Json(RespuestaLista {
        count: data.len(),
        data,
    }))
}

pub async fn crear_clave(
    State(estado): State<Arc<Estado>>,
    identidad: Identidad,
    Json(peticion): Json<PeticionCrearClave>,
) -> Result<impl IntoResponse, ErrorApi> {
    let nombre = peticion.name.trim();
    if nombre.is_empty() {
        return Err(ErrorApi::Peticion(
            "el nombre de la clave no puede estar vacio".into(),
        ));
    }

    let (clave, token) = ApiKey::emitir(identidad.organizacion, nombre, estado.reloj.ahora());
    let respuesta = RespuestaClaveCreada {
        key_id: clave.id,
        name: clave.nombre.clone(),
        prefix: clave.prefijo.clone(),
        token,
        created_at: clave.creada_en,
    };
    estado.cuentas.crear_api_key(clave).await?;
    Ok((StatusCode::CREATED, Json(respuesta)))
}

pub async fn listar_claves(
    State(estado): State<Arc<Estado>>,
    identidad: Identidad,
) -> Result<impl IntoResponse, ErrorApi> {
    let claves = estado
        .cuentas
        .listar_api_keys(identidad.organizacion)
        .await?;
    let data: Vec<RespuestaClave> = claves.iter().map(RespuestaClave::from).collect();
    Ok(Json(RespuestaLista {
        count: data.len(),
        data,
    }))
}

pub async fn revocar_clave(
    State(estado): State<Arc<Estado>>,
    identidad: Identidad,
    Path(id): Path<IdApiKey>,
) -> Result<impl IntoResponse, ErrorApi> {
    estado
        .cuentas
        .revocar_api_key(identidad.organizacion, id)
        .await
        .map_err(|error| match error {
            dp_dominio::ErrorRepositorio::NoEncontrado { .. } => ErrorApi::ClaveNoEncontrada(id),
            otro => ErrorApi::from(otro),
        })?;
    Ok(StatusCode::NO_CONTENT)
}

impl From<&ApiKey> for RespuestaClave {
    fn from(clave: &ApiKey) -> Self {
        RespuestaClave {
            key_id: clave.id,
            name: clave.nombre.clone(),
            prefix: clave.prefijo.clone(),
            revoked: clave.revocada,
            last_used_at: clave.ultimo_uso,
            created_at: clave.creada_en,
        }
    }
}
