//! Los tipos de datos del motor.
//!
//! Aquí ya no hay `ClienteBruto` ni `ClienteLimpio`. Existían cuando el motor
//! solo sabía procesar clientes; ahora las filas se leen por nombre de columna
//! (ver `fila.rs`), así que el esquema lo pone el archivo del cliente y no
//! nuestro código.

use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum EstadoEmail {
    Valido,
    Vacio,
    SinArroba,
    SinUsuario,
    SinDominio,
}

impl EstadoEmail {
    /// `&'static str` y no `&str`: el texto está incrustado en el binario y
    /// vive todo el programa, así que la referencia no queda atada a `&self`.
    /// Eso permite usarla después de que el préstamo de self haya terminado.
    pub fn etiqueta(&self) -> &'static str {
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
///
/// `Copy` porque son siete enteros: copiarlo cuesta menos que pasar una
/// referencia y evita tener que pensar en préstamos al guardarlo dentro de un
/// `Job`. `Deserialize` porque en la fase 2 este resumen se guarda como JSON
/// en PostgreSQL y hay que poder leerlo de vuelta.
#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Resumen {
    pub leidas: u64,
    pub escritas: u64,
    pub duplicadas: u64,
    pub emails_invalidos: u64,
    pub telefonos_invalidos: u64,
    pub telefonos_vacios: u64,
    pub campos_normalizados: u64,
}
