//! Acceso a las columnas del CSV por nombre.
//!
//! Antes leíamos cada fila hacia la struct fija `ClienteBruto`, lo que ataba el
//! motor a un único esquema. Un cliente que sube facturas con otras columnas no
//! cabía. Ahora resolvemos los nombres a índices UNA vez, al leer el
//! encabezado, y después trabajamos con posiciones enteras: igual de rápido que
//! la struct, pero sirviendo cualquier esquema.

use crate::error::{ErrorDp, Resultado};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Encabezados {
    nombres: Vec<String>,
    /// Nombre de columna a su posición. El mapa existe solo para resolver la
    /// configuración al arrancar; en el bucle de filas no se toca.
    indices: HashMap<String, usize>,
}

impl Encabezados {
    pub fn desde_csv(registro: &csv::StringRecord) -> Self {
        let nombres: Vec<String> = registro.iter().map(|c| c.trim().to_string()).collect();
        let indices = nombres
            .iter()
            .enumerate()
            .map(|(posicion, nombre)| (nombre.clone(), posicion))
            .collect();

        Encabezados { nombres, indices }
    }

    pub fn nombres(&self) -> &[String] {
        &self.nombres
    }

    /// Falla con el nombre exacto que falta, que es la mitad del valor de un
    /// buen mensaje de error.
    pub fn indice(&self, nombre: &str) -> Resultado<usize> {
        self.indices
            .get(nombre)
            .copied()
            .ok_or_else(|| ErrorDp::ColumnaFaltante(nombre.to_string()))
    }

    /// Resuelve varios nombres de golpe.
    ///
    /// El truco está en el `collect`: un iterador de `Resultado<usize>` se
    /// puede recoger en un `Resultado<Vec<usize>>`. Si algún elemento es Err,
    /// el collect corta ahí mismo y devuelve ese error; si todos son Ok,
    /// devuelve el Vec. Reemplaza un bucle con `match` dentro.
    pub fn indices_de(&self, nombres: &[String]) -> Resultado<Vec<usize>> {
        nombres.iter().map(|n| self.indice(n)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encabezados_de_prueba() -> Encabezados {
        Encabezados::desde_csv(&csv::StringRecord::from(vec![
            "nombre", " email ", "telefono",
        ]))
    }

    #[test]
    fn los_nombres_del_encabezado_se_recortan() {
        let enc = encabezados_de_prueba();
        assert_eq!(enc.nombres(), ["nombre", "email", "telefono"]);
        assert_eq!(enc.indice("email").unwrap(), 1);
    }

    #[test]
    fn una_columna_que_no_existe_da_error_con_su_nombre() {
        let enc = encabezados_de_prueba();
        let error = enc.indice("nit").expect_err("no deberia encontrar 'nit'");
        assert_eq!(error.codigo(), "E_COLUMNA_FALTANTE");
        assert!(error.to_string().contains("nit"));
    }

    #[test]
    fn indices_de_corta_en_la_primera_columna_faltante() {
        let enc = encabezados_de_prueba();
        let pedidas = vec!["email".to_string(), "ciudad".to_string()];
        let error = enc
            .indices_de(&pedidas)
            .expect_err("deberia fallar por 'ciudad'");
        assert!(error.to_string().contains("ciudad"));
    }

    #[test]
    fn indices_de_resuelve_varias_columnas_en_orden() {
        let enc = encabezados_de_prueba();
        let pedidas = vec!["telefono".to_string(), "nombre".to_string()];
        assert_eq!(enc.indices_de(&pedidas).unwrap(), vec![2, 0]);
    }
}
