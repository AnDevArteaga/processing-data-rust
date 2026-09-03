//! URLs temporales firmadas.
//!
//! La sección 9 del PDF exige URLs firmadas y temporales para subir y
//! descargar. En la fase 4 las firmará S3; hoy las firmamos nosotros, con el
//! mismo contrato: una URL que caduca y sirve para una sola acción sobre un
//! solo recurso.
//!
//! Cuatro detalles que hacen que esto sea seguro y no decorativo:
//!
//! 1. La firma cubre la acción **y** el recurso. Una URL de descarga no sirve
//!    para subir, y la de un archivo no sirve para otro.
//! 2. La organización va dentro de lo firmado. Sin eso, cambiar el parámetro
//!    `org` de la URL alcanzaría el archivo de otro cliente.
//! 3. El vencimiento va dentro de lo firmado, así que no se alarga editando
//!    la URL.
//! 4. El tamaño máximo autorizado también va firmado. Es lo mismo que hace una
//!    política prefirmada de S3 con su `content-length-range`: quien emite la
//!    URL conoce el plan del cliente, así que el límite se decide ahí y el
//!    endpoint de subida solo lo obedece.
//!
//! La comparación de firmas es de tiempo constante. Con `==`, la diferencia de
//! tiempo revelaría cuántos bytes iniciales acertó un atacante, y con eso se
//! adivina una firma byte a byte.

use crate::error::ErrorApi;
use chrono::{DateTime, Duration, Utc};
// `KeyInit` aporta `new_from_slice` y `Mac` el resto. En la familia de crates
// de RustCrypto la funcionalidad viene repartida en traits, así que hay que
// importar los dos aunque solo se use un tipo.
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use std::fmt::Display;

type HmacSha256 = Hmac<Sha256>;

/// Cuánto vive una URL temporal. Corto a propósito: es una credencial.
pub const VIGENCIA_MINUTOS: i64 = 15;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accion {
    Subir,
    Descargar,
}

impl Accion {
    fn etiqueta(&self) -> &'static str {
        match self {
            Accion::Subir => "subir",
            Accion::Descargar => "descargar",
        }
    }
}

pub struct Firmante {
    secreto: Vec<u8>,
}

impl Firmante {
    pub fn nuevo(secreto: &str) -> Self {
        Firmante {
            secreto: secreto.as_bytes().to_vec(),
        }
    }

    /// El recurso de una subida incluye el tope de bytes autorizado.
    pub fn recurso_subida(
        organizacion: &impl Display,
        archivo: &impl Display,
        max_bytes: u64,
    ) -> String {
        format!("{organizacion}/{archivo}/{max_bytes}")
    }

    pub fn recurso_descarga(organizacion: &impl Display, archivo: &impl Display) -> String {
        format!("{organizacion}/{archivo}")
    }

    /// Firma una acción sobre un recurso. Devuelve la firma y el vencimiento.
    pub fn firmar(&self, accion: Accion, recurso: &str, ahora: DateTime<Utc>) -> (String, i64) {
        let expira = (ahora + Duration::minutes(VIGENCIA_MINUTOS)).timestamp();
        (
            hex::encode(self.etiqueta(accion, recurso, expira)),
            expira,
        )
    }

    pub fn verificar(
        &self,
        accion: Accion,
        recurso: &str,
        expira: i64,
        firma_recibida: &str,
        ahora: DateTime<Utc>,
    ) -> Result<(), ErrorApi> {
        // El vencimiento primero: no tiene sentido calcular el HMAC de una URL
        // que ya caducó.
        if ahora.timestamp() >= expira {
            return Err(ErrorApi::FirmaVencida);
        }

        let recibida = hex::decode(firma_recibida).map_err(|_| ErrorApi::FirmaInvalida)?;

        let mut mac = self.mac();
        mac.update(Self::mensaje(accion, recurso, expira).as_bytes());

        // `verify_slice` compara en tiempo constante. Es la línea que impide
        // adivinar la firma midiendo cuánto tarda la respuesta.
        mac.verify_slice(&recibida)
            .map_err(|_| ErrorApi::FirmaInvalida)
    }

    /// Cuándo vence una firma emitida ahora.
    pub fn vencimiento(ahora: DateTime<Utc>) -> DateTime<Utc> {
        ahora + Duration::minutes(VIGENCIA_MINUTOS)
    }

    fn mac(&self) -> HmacSha256 {
        HmacSha256::new_from_slice(&self.secreto)
            .expect("HMAC-SHA256 acepta claves de cualquier longitud")
    }

    fn mensaje(accion: Accion, recurso: &str, expira: i64) -> String {
        // El separador `:` basta porque ninguna de las partes puede
        // contenerlo: la acción es una constante, el recurso está compuesto de
        // identificadores y números separados por `/`, y el vencimiento es un
        // entero.
        format!("{}:{}:{}", accion.etiqueta(), recurso, expira)
    }

    fn etiqueta(&self, accion: Accion, recurso: &str, expira: i64) -> Vec<u8> {
        let mut mac = self.mac();
        mac.update(Self::mensaje(accion, recurso, expira).as_bytes());
        mac.finalize().into_bytes().to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn firmante() -> Firmante {
        Firmante::nuevo("secreto-de-prueba")
    }

    fn descarga_mia() -> String {
        Firmante::recurso_descarga(&"org_mia", &"file_abc")
    }

    #[test]
    fn una_firma_recien_hecha_se_verifica() {
        let f = firmante();
        let ahora = Utc::now();
        let recurso = descarga_mia();

        let (firma, expira) = f.firmar(Accion::Descargar, &recurso, ahora);
        assert!(
            f.verificar(Accion::Descargar, &recurso, expira, &firma, ahora)
                .is_ok()
        );
    }

    /// Una URL de descarga no debe servir para subir. Sin esto, cualquiera con
    /// un enlace de descarga podría sobrescribir el archivo.
    #[test]
    fn la_firma_no_sirve_para_otra_accion() {
        let f = firmante();
        let ahora = Utc::now();
        let recurso = descarga_mia();

        let (firma, expira) = f.firmar(Accion::Descargar, &recurso, ahora);
        let error = f
            .verificar(Accion::Subir, &recurso, expira, &firma, ahora)
            .expect_err("no deberia valer para subir");

        assert!(matches!(error, ErrorApi::FirmaInvalida));
    }

    #[test]
    fn la_firma_no_sirve_para_otro_archivo() {
        let f = firmante();
        let ahora = Utc::now();

        let (firma, expira) = f.firmar(Accion::Descargar, &descarga_mia(), ahora);
        let otro = Firmante::recurso_descarga(&"org_mia", &"file_otro");

        assert!(
            f.verificar(Accion::Descargar, &otro, expira, &firma, ahora)
                .is_err()
        );
    }

    /// El ataque directo: tomar una firma válida y cambiar la organización de
    /// la URL para alcanzar el archivo de otro cliente.
    #[test]
    fn cambiar_la_organizacion_de_la_url_invalida_la_firma() {
        let f = firmante();
        let ahora = Utc::now();

        let (firma, expira) = f.firmar(Accion::Descargar, &descarga_mia(), ahora);
        let ajena = Firmante::recurso_descarga(&"org_ajena", &"file_abc");

        assert!(
            f.verificar(Accion::Descargar, &ajena, expira, &firma, ahora)
                .is_err()
        );
    }

    /// El límite de tamaño está firmado, así que un cliente del plan gratuito
    /// no puede pedirse a sí mismo permiso para subir un giga.
    #[test]
    fn subir_el_tope_de_bytes_en_la_url_invalida_la_firma() {
        let f = firmante();
        let ahora = Utc::now();

        let autorizado = Firmante::recurso_subida(&"org_mia", &"file_abc", 5 * 1024 * 1024);
        let (firma, expira) = f.firmar(Accion::Subir, &autorizado, ahora);

        let inflado = Firmante::recurso_subida(&"org_mia", &"file_abc", 1024 * 1024 * 1024);
        let error = f
            .verificar(Accion::Subir, &inflado, expira, &firma, ahora)
            .expect_err("no deberia poder subirse el limite");

        assert!(matches!(error, ErrorApi::FirmaInvalida));
    }

    #[test]
    fn una_firma_vencida_se_rechaza() {
        let f = firmante();
        let ahora = Utc::now();
        let recurso = descarga_mia();

        let (firma, expira) = f.firmar(Accion::Descargar, &recurso, ahora);
        let despues = ahora + Duration::minutes(VIGENCIA_MINUTOS + 1);

        let error = f
            .verificar(Accion::Descargar, &recurso, expira, &firma, despues)
            .expect_err("deberia haber vencido");
        assert!(matches!(error, ErrorApi::FirmaVencida));
    }

    /// Alargar el vencimiento editando la URL invalida la firma, porque el
    /// vencimiento es parte de lo firmado.
    #[test]
    fn estirar_el_vencimiento_invalida_la_firma() {
        let f = firmante();
        let ahora = Utc::now();
        let recurso = descarga_mia();

        let (firma, expira) = f.firmar(Accion::Descargar, &recurso, ahora);
        let estirado = expira + 86_400;

        let error = f
            .verificar(Accion::Descargar, &recurso, estirado, &firma, ahora)
            .expect_err("no deberia aceptar el vencimiento alterado");
        assert!(matches!(error, ErrorApi::FirmaInvalida));
    }

    #[test]
    fn otra_clave_secreta_no_puede_firmar_por_nosotros() {
        let nuestro = firmante();
        let ajeno = Firmante::nuevo("otro-secreto");
        let ahora = Utc::now();
        let recurso = descarga_mia();

        let (firma, expira) = ajeno.firmar(Accion::Descargar, &recurso, ahora);
        assert!(
            nuestro
                .verificar(Accion::Descargar, &recurso, expira, &firma, ahora)
                .is_err()
        );
    }

    #[test]
    fn una_firma_que_no_es_hexadecimal_se_rechaza_sin_panico() {
        let f = firmante();
        let ahora = Utc::now();
        let expira = (ahora + Duration::minutes(5)).timestamp();

        let error = f
            .verificar(
                Accion::Descargar,
                &descarga_mia(),
                expira,
                "no-es-hex!!",
                ahora,
            )
            .expect_err("deberia rechazarla");
        assert!(matches!(error, ErrorApi::FirmaInvalida));
    }

    #[test]
    fn el_vencimiento_respeta_la_vigencia_configurada() {
        let ahora = Utc::now();
        assert_eq!(
            Firmante::vencimiento(ahora),
            ahora + Duration::minutes(VIGENCIA_MINUTOS)
        );
    }
}
