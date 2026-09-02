// `crate::` en el `use` de abajo significa "desde la raíz de esta librería".
// Las otras dos rutas posibles son `super::` (el módulo padre) y `self::`
// (este mismo módulo).
use crate::modelo::{ClienteBruto, ClienteLimpio, EstadoEmail, Resumen};

pub fn normalizar_email(bruto: &str) -> String {
    bruto.trim().to_lowercase()
}

pub fn validar_email(email: &str) -> EstadoEmail {
    if email.is_empty() {
        return EstadoEmail::Vacio;
    }
    match email.split_once('@') {
        None => EstadoEmail::SinArroba,
        Some((usuario, dominio)) => {
            if usuario.is_empty() {
                EstadoEmail::SinUsuario
            } else if dominio.is_empty() || !dominio.contains('.') {
                EstadoEmail::SinDominio
            } else {
                EstadoEmail::Valido
            }
        }
    }
}

/// Normaliza un teléfono colombiano a 10 dígitos.
/// `None` significa que el dato no sirve.
pub fn normalizar_telefono(bruto: &str) -> Option<String> {
    let digitos: String = bruto.chars().filter(|c| c.is_ascii_digit()).collect();
    if digitos.is_empty() {
        return None;
    }
    let numero = if digitos.len() == 12 && digitos.starts_with("57") {
        &digitos[2..]
    } else {
        &digitos[..]
    };
    if numero.len() == 10 && numero.starts_with('3') {
        Some(numero.to_string())
    } else {
        None
    }
}

/// Transforma una fila cruda en una limpia y actualiza los contadores.
pub fn procesar_fila(bruto: &ClienteBruto, r: &mut Resumen) -> ClienteLimpio {
    let email = normalizar_email(&bruto.email);
    if email != bruto.email {
        r.campos_normalizados += 1;
    }

    let estado = validar_email(&email);
    if estado != EstadoEmail::Valido {
        r.emails_invalidos += 1;
    }

    let telefono = normalizar_telefono(&bruto.telefono);
    match &telefono {
        Some(numero) => {
            if *numero != bruto.telefono {
                r.campos_normalizados += 1;
            }
        }
        None => {
            if bruto.telefono.trim().is_empty() {
                r.telefonos_vacios += 1;
            } else {
                r.telefonos_invalidos += 1;
            }
        }
    }

    ClienteLimpio {
        nombre: bruto.nombre.trim().to_string(),
        ciudad: bruto.ciudad.trim().to_string(),
        email_estado: estado.etiqueta().to_string(),
        telefono_valido: telefono.is_some(),
        email,
        telefono,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_se_normaliza_a_minusculas_y_sin_espacios() {
        assert_eq!(
            normalizar_email("  MARIA@Hotmail.COM  "),
            "maria@hotmail.com"
        );
    }

    #[test]
    fn email_ya_limpio_no_cambia() {
        assert_eq!(normalizar_email("juan@gmail.com"), "juan@gmail.com");
    }

    #[test]
    fn email_valido_se_acepta() {
        assert_eq!(validar_email("juan@gmail.com"), EstadoEmail::Valido);
    }

    #[test]
    fn email_sin_arroba_se_rechaza() {
        assert_eq!(
            validar_email("carlos-arroba-gmail.com"),
            EstadoEmail::SinArroba
        );
    }

    #[test]
    fn email_sin_dominio_se_rechaza() {
        assert_eq!(validar_email("sofia@"), EstadoEmail::SinDominio);
    }

    #[test]
    fn email_sin_usuario_se_rechaza() {
        assert_eq!(validar_email("@gmail.com"), EstadoEmail::SinUsuario);
    }

    #[test]
    fn email_vacio_se_rechaza() {
        assert_eq!(validar_email(""), EstadoEmail::Vacio);
    }

    #[test]
    fn dominio_sin_punto_se_rechaza() {
        assert_eq!(validar_email("juan@localhost"), EstadoEmail::SinDominio);
    }

    #[test]
    fn telefono_de_diez_digitos_pasa_igual() {
        assert_eq!(
            normalizar_telefono("3001234567"),
            Some("3001234567".to_string())
        );
    }

    #[test]
    fn telefono_con_espacios_se_limpia() {
        assert_eq!(
            normalizar_telefono("300 123 4567"),
            Some("3001234567".to_string())
        );
    }

    #[test]
    fn telefono_con_indicativo_pierde_el_57() {
        assert_eq!(
            normalizar_telefono("+57 301 555 1122"),
            Some("3015551122".to_string())
        );
    }

    #[test]
    fn telefono_vacio_es_none() {
        assert_eq!(normalizar_telefono(""), None);
        assert_eq!(normalizar_telefono("   "), None);
    }

    #[test]
    fn telefono_demasiado_corto_es_none() {
        assert_eq!(normalizar_telefono("12345"), None);
    }

    #[test]
    fn telefono_fijo_que_no_empieza_en_tres_es_none() {
        assert_eq!(normalizar_telefono("6012345678"), None);
    }

    #[test]
    fn procesar_fila_cuenta_las_normalizaciones() {
        let bruto = ClienteBruto {
            nombre: "  Juan Pérez  ".to_string(),
            email: "  JUAN@Gmail.com ".to_string(),
            telefono: "+57 300 123 4567".to_string(),
            ciudad: " Bogotá ".to_string(),
        };
        let mut r = Resumen::default();
        let limpio = procesar_fila(&bruto, &mut r);

        assert_eq!(limpio.nombre, "Juan Pérez");
        assert_eq!(limpio.email, "juan@gmail.com");
        assert_eq!(limpio.telefono, Some("3001234567".to_string()));
        assert_eq!(limpio.ciudad, "Bogotá");
        assert!(limpio.telefono_valido);
        // Se normalizaron el email y el teléfono: dos campos.
        assert_eq!(r.campos_normalizados, 2);
        assert_eq!(r.emails_invalidos, 0);
    }
}
