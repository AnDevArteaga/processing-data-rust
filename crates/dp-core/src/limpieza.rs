//! La configuración de un job y el motor que limpia cada fila.

use crate::error::Resultado;
use crate::fila::Encabezados;
use crate::modelo::{EstadoEmail, Resumen};
use crate::normalizacion::{normalizar_email, normalizar_telefono, validar_email};

/// Tope de claves únicas por defecto. Un millón de claves cuesta unos 85 MB de
/// índice, que es lo máximo que queremos gastar en un worker compartido. Los
/// planes de pago lo suben.
pub const LIMITE_CLAVES_POR_DEFECTO: usize = 1_000_000;

/// Separador de las partes de una clave compuesta.
///
/// Es el carácter de control ASCII 0x1F, *Unit Separator*, inventado justo para
/// esto. La razón de no usar `|` o `,` es que la clave sería ambigua: con una
/// concatenación simple, las filas `("ab", "c")` y `("a", "bc")` producen la
/// misma clave `abc` y borraríamos una fila legítima. Y cualquier separador
/// imprimible puede aparecer dentro de los datos del cliente.
pub const SEPARADOR_CLAVE: char = '\u{1F}';

/// Cómo tratar cada columna. `Copy` porque es del tamaño de un byte: copiarlo
/// es más barato que pasar una referencia.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Tipo {
    Texto,
    Email,
    Telefono,
}

/// Los parámetros de un job, que en producción llegarán en el payload de
/// `POST /v1/jobs` en vez de por la línea de comandos.
#[derive(Debug, Clone)]
pub struct Config {
    /// Columnas que forman la clave de deduplicación, en orden.
    /// Vacío significa "no deduplicar".
    pub claves: Vec<String>,
    /// Columnas a normalizar y validar como email.
    pub emails: Vec<String>,
    /// Columnas a normalizar como teléfono.
    pub telefonos: Vec<String>,
    /// Cuántas claves únicas puede recordar antes de abortar el job.
    pub limite_claves: usize,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            claves: vec!["email".to_string()],
            emails: vec!["email".to_string()],
            telefonos: vec!["telefono".to_string()],
            limite_claves: LIMITE_CLAVES_POR_DEFECTO,
        }
    }
}

/// Traduce la configuración (nombres de columna) a posiciones, una sola vez.
///
/// `Debug` no es decorativo: `expect_err` en un test necesita poder imprimir el
/// valor de éxito para decirte qué recibió cuando esperaba un error, y sin
/// `Debug` el compilador rechaza esa llamada.
#[derive(Debug)]
pub struct Limpiador {
    /// Un tipo por columna del archivo, indexado por posición.
    tipos: Vec<Tipo>,
    /// Posiciones que forman la clave, en el orden que pidió el cliente.
    clave: Vec<usize>,
    /// El encabezado que verá el archivo de salida.
    columnas_salida: Vec<String>,
}

impl Limpiador {
    /// Aquí es donde se valida el esquema: si el cliente pide deduplicar por
    /// una columna que no existe, el job muere ahora y no después de haber
    /// procesado medio archivo.
    pub fn nuevo(encabezados: &Encabezados, config: &Config) -> Resultado<Self> {
        let mut tipos = vec![Tipo::Texto; encabezados.nombres().len()];

        for nombre in &config.emails {
            tipos[encabezados.indice(nombre)?] = Tipo::Email;
        }
        for nombre in &config.telefonos {
            tipos[encabezados.indice(nombre)?] = Tipo::Telefono;
        }

        let clave = encabezados.indices_de(&config.claves)?;

        // A las columnas originales les añadimos una de diagnóstico por cada
        // email y cada teléfono.
        let mut columnas_salida = encabezados.nombres().to_vec();
        for (posicion, tipo) in tipos.iter().enumerate() {
            let nombre = &encabezados.nombres()[posicion];
            match tipo {
                Tipo::Email => columnas_salida.push(format!("{nombre}_estado")),
                Tipo::Telefono => columnas_salida.push(format!("{nombre}_valido")),
                Tipo::Texto => {}
            }
        }

        Ok(Limpiador {
            tipos,
            clave,
            columnas_salida,
        })
    }

    pub fn columnas_salida(&self) -> &[String] {
        &self.columnas_salida
    }

    pub fn deduplica(&self) -> bool {
        !self.clave.is_empty()
    }

    /// Escribe la clave de deduplicación en `clave`, reutilizando su memoria.
    ///
    /// Recibe `&mut String` en vez de devolver un String nuevo para no pedirle
    /// memoria al sistema en cada una de los millones de filas: el buffer se
    /// asigna una vez y se limpia, conservando su capacidad.
    pub fn construir_clave(&self, fila: &csv::StringRecord, clave: &mut String) {
        clave.clear();

        for (orden, &posicion) in self.clave.iter().enumerate() {
            if orden > 0 {
                clave.push(SEPARADOR_CLAVE);
            }

            let valor = fila.get(posicion).unwrap_or("");
            match self.tipos[posicion] {
                // `char::to_lowercase` devuelve un ITERADOR de chars, no un
                // char, porque hay letras que al minusculizarse se convierten
                // en varias. `flat_map` los aplana y `extend` los mete al
                // String sin crear ninguno intermedio.
                Tipo::Email | Tipo::Texto => {
                    clave.extend(valor.trim().chars().flat_map(char::to_lowercase));
                }
                Tipo::Telefono => {
                    if let Some(numero) = normalizar_telefono(valor) {
                        clave.push_str(&numero);
                    }
                }
            }
        }
    }

    /// Normaliza la fila, cuenta los problemas en `r` y deja el resultado en
    /// `salida` (también reutilizada entre filas).
    pub fn limpiar(&self, fila: &csv::StringRecord, r: &mut Resumen, salida: &mut Vec<String>) {
        salida.clear();

        // Primera pasada: normalizar cada celda según su tipo.
        for (posicion, valor) in fila.iter().enumerate() {
            match self.tipos.get(posicion) {
                Some(Tipo::Email) => {
                    let email = normalizar_email(valor);
                    if email != valor {
                        r.campos_normalizados += 1;
                    }
                    salida.push(email);
                }
                Some(Tipo::Telefono) => match normalizar_telefono(valor) {
                    Some(numero) => {
                        if numero != valor {
                            r.campos_normalizados += 1;
                        }
                        salida.push(numero);
                    }
                    None => {
                        if valor.trim().is_empty() {
                            r.telefonos_vacios += 1;
                        } else {
                            r.telefonos_invalidos += 1;
                        }
                        // Un teléfono que no sirve sale vacío, no basura.
                        salida.push(String::new());
                    }
                },
                _ => salida.push(valor.trim().to_string()),
            }
        }

        // Segunda pasada: las columnas de diagnóstico, leyendo los valores ya
        // normalizados de la primera. Van al final, en el mismo orden en que
        // `nuevo` construyó el encabezado.
        for posicion in 0..self.tipos.len() {
            match self.tipos[posicion] {
                Tipo::Email => {
                    // `etiqueta()` devuelve `&'static str`, así que el préstamo
                    // de `salida` termina en esta línea y el `push` siguiente
                    // puede volver a tomarla prestada como mutable.
                    let estado = validar_email(&salida[posicion]);
                    if estado != EstadoEmail::Valido {
                        r.emails_invalidos += 1;
                    }
                    let etiqueta = estado.etiqueta();
                    salida.push(etiqueta.to_string());
                }
                Tipo::Telefono => {
                    let valido = !salida[posicion].is_empty();
                    salida.push(valido.to_string());
                }
                Tipo::Texto => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limpiador_de_prueba(config: &Config) -> Limpiador {
        let encabezados = Encabezados::desde_csv(&csv::StringRecord::from(vec![
            "nombre", "email", "telefono", "ciudad",
        ]));
        Limpiador::nuevo(&encabezados, config).expect("la configuracion deberia resolver")
    }

    fn fila(campos: &[&str]) -> csv::StringRecord {
        csv::StringRecord::from(campos.to_vec())
    }

    #[test]
    fn el_encabezado_de_salida_agrega_las_columnas_de_diagnostico() {
        let limpiador = limpiador_de_prueba(&Config::default());
        assert_eq!(
            limpiador.columnas_salida(),
            [
                "nombre",
                "email",
                "telefono",
                "ciudad",
                "email_estado",
                "telefono_valido"
            ]
        );
    }

    #[test]
    fn limpiar_normaliza_y_cuenta_igual_que_antes() {
        let limpiador = limpiador_de_prueba(&Config::default());
        let mut r = Resumen::default();
        let mut salida = Vec::new();

        limpiador.limpiar(
            &fila(&[
                "  Juan Pérez  ",
                "  JUAN@Gmail.com ",
                "+57 300 123 4567",
                " Bogotá ",
            ]),
            &mut r,
            &mut salida,
        );

        assert_eq!(
            salida,
            [
                "Juan Pérez",
                "juan@gmail.com",
                "3001234567",
                "Bogotá",
                "ok",
                "true"
            ]
        );
        assert_eq!(r.campos_normalizados, 2);
        assert_eq!(r.emails_invalidos, 0);
    }

    #[test]
    fn un_telefono_invalido_sale_vacio_y_se_cuenta() {
        let limpiador = limpiador_de_prueba(&Config::default());
        let mut r = Resumen::default();
        let mut salida = Vec::new();

        limpiador.limpiar(
            &fila(&["Ana", "ana@x.com", "12345", "Cali"]),
            &mut r,
            &mut salida,
        );

        assert_eq!(salida[2], "");
        assert_eq!(salida[5], "false");
        assert_eq!(r.telefonos_invalidos, 1);
        assert_eq!(r.telefonos_vacios, 0);
    }

    #[test]
    fn una_clave_de_una_columna_se_normaliza() {
        let limpiador = limpiador_de_prueba(&Config::default());
        let mut clave = String::new();

        limpiador.construir_clave(
            &fila(&["Ana", " ANA@X.com ", "3001234567", "Cali"]),
            &mut clave,
        );
        assert_eq!(clave, "ana@x.com");
    }

    /// El caso que motivó todo el hallazgo 2: dos personas con el mismo
    /// teléfono pero emails distintos ya no se consideran la misma.
    #[test]
    fn una_clave_de_dos_columnas_usa_el_separador_de_unidad() {
        let config = Config {
            claves: vec!["email".to_string(), "telefono".to_string()],
            ..Config::default()
        };
        let limpiador = limpiador_de_prueba(&config);
        let mut clave = String::new();

        limpiador.construir_clave(
            &fila(&["Ana", "ANA@x.com", "+573001234567", "Cali"]),
            &mut clave,
        );
        assert_eq!(clave, format!("ana@x.com{SEPARADOR_CLAVE}3001234567"));
    }

    /// La ambigüedad que evita el separador: sin él, estas dos filas
    /// producirían la misma clave y una se borraría.
    #[test]
    fn el_separador_evita_que_dos_filas_distintas_choquen() {
        let config = Config {
            claves: vec!["nombre".to_string(), "ciudad".to_string()],
            emails: vec![],
            telefonos: vec![],
            ..Config::default()
        };
        let limpiador = limpiador_de_prueba(&config);

        let mut primera = String::new();
        let mut segunda = String::new();
        limpiador.construir_clave(&fila(&["ab", "", "", "c"]), &mut primera);
        limpiador.construir_clave(&fila(&["a", "", "", "bc"]), &mut segunda);

        assert_ne!(primera, segunda);
    }

    #[test]
    fn una_clave_por_columna_inexistente_falla_al_construir() {
        let config = Config {
            claves: vec!["nit".to_string()],
            ..Config::default()
        };
        let encabezados = Encabezados::desde_csv(&csv::StringRecord::from(vec![
            "nombre", "email", "telefono", "ciudad",
        ]));
        let error = Limpiador::nuevo(&encabezados, &config).expect_err("no existe la columna nit");
        assert_eq!(error.codigo(), "E_COLUMNA_FALTANTE");
    }

    #[test]
    fn sin_claves_no_se_deduplica() {
        let config = Config {
            claves: vec![],
            ..Config::default()
        };
        assert!(!limpiador_de_prueba(&config).deduplica());
        assert!(limpiador_de_prueba(&Config::default()).deduplica());
    }
}
