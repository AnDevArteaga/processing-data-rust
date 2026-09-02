use serde::{Deserialize, Serialize};

/// Cómo viene cada fila del CSV de entrada.
///
/// Los campos también llevan `pub`: hacer pública la struct no hace públicos
/// sus campos. Son dos permisos separados.
#[derive(Debug, Deserialize)]
pub struct ClienteBruto {
    pub nombre: String,
    pub email: String,
    pub telefono: String,
    pub ciudad: String,
}

/// Cómo sale cada fila ya procesada.
#[derive(Debug, Serialize)]
pub struct ClienteLimpio {
    pub nombre: String,
    pub email: String,
    pub telefono: Option<String>,
    pub ciudad: String,
    pub email_estado: String,
    pub telefono_valido: bool,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum EstadoEmail {
    Valido,
    Vacio,
    SinArroba,
    SinUsuario,
    SinDominio,
}

impl EstadoEmail {
    pub fn etiqueta(&self) -> &str {
        match self {
            EstadoEmail::Valido => "ok",
            EstadoEmail::Vacio => "vacio",
            EstadoEmail::SinArroba => "sin_arroba",
            EstadoEmail::SinUsuario => "sin_usuario",
            EstadoEmail::SinDominio => "sin_dominio",
        }
    }
}

/// El resultado que la API devuelve al cliente (sección 4.5 del PDF).
#[derive(Debug, Default, Serialize, PartialEq)]
pub struct Resumen {
    pub leidas: u64,
    pub escritas: u64,
    pub duplicadas: u64,
    pub emails_invalidos: u64,
    pub telefonos_invalidos: u64,
    pub telefonos_vacios: u64,
    pub campos_normalizados: u64,
}
