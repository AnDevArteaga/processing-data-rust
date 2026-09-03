//! Los errores del dominio y de los puertos.

use crate::job::EstadoJob;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ErrorAlmacen {
    #[error("el objeto '{0}' no existe en el almacen")]
    NoExiste(String),

    #[error("fallo de entrada/salida en el almacen: {0}")]
    Io(#[from] std::io::Error),

    /// La clave la construimos nosotros a partir de UUID, así que si esto
    /// salta es un bug nuestro, no un ataque. Aun así se valida: es la última
    /// línea de defensa contra escribir fuera del directorio del almacén.
    #[error("la clave de almacen '{0}' no es valida")]
    ClaveInvalida(String),
}

#[derive(Debug, Error)]
pub enum ErrorRepositorio {
    #[error("no existe el {tipo} con identificador '{id}'")]
    NoEncontrado { tipo: &'static str, id: String },

    #[error("transicion de estado invalida: {0}")]
    Transicion(#[from] TransicionInvalida),

    #[error("fallo del almacenamiento de datos: {0}")]
    Interno(String),
}

impl ErrorRepositorio {
    pub fn no_encontrado(tipo: &'static str, id: impl std::fmt::Display) -> Self {
        ErrorRepositorio::NoEncontrado {
            tipo,
            id: id.to_string(),
        }
    }
}

/// Se intentó mover un job a un estado al que no puede ir desde donde está.
///
/// Este tipo existe para que la máquina de estados sea un invariante del
/// dominio y no una convención que cada llamador recuerde respetar. Completar
/// un job ya cancelado, por ejemplo, debe ser imposible, no improbable.
#[derive(Debug, Error, PartialEq)]
#[error("un job en estado {desde} no puede pasar a {hacia}")]
pub struct TransicionInvalida {
    pub desde: EstadoJob,
    pub hacia: EstadoJob,
}
