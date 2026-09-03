// `crate::` en el `use` de abajo significa "desde la raíz de esta librería".
// Las otras dos rutas posibles son `super::` (el módulo padre) y `self::`
// (este mismo módulo).
use crate::modelo::EstadoEmail;

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

// `procesar_fila` vivía aquí y devolvía un `ClienteLimpio`. Se movió a
// `limpieza.rs` como `Limpiador::limpiar`, que trabaja sobre columnas
// arbitrarias en vez de sobre una struct fija de clientes. Este módulo se
// queda con las funciones puras: entran datos, salen datos, no tocan nada.

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

    // El test de fila completa se mudó a `limpieza.rs`, junto con la lógica.
}
