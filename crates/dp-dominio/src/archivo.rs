//! Metadata de archivos. Los bytes viven en el almacén, nunca aquí.

use crate::ids::{IdArchivo, IdOrganizacion};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// Tipo real del archivo, deducido de su contenido.
///
/// Sección 9 del PDF: "nunca confiar en nombre/extensión MIME enviados por el
/// usuario". Un `.csv` que en realidad es un ejecutable no debe llegar al
/// motor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoArchivo {
    Csv,
    Pdf,
    Json,
    Desconocido,
}

impl TipoArchivo {
    /// Detecta el tipo leyendo los primeros bytes.
    ///
    /// No es antivirus ni pretende serlo: es la validación mínima para que una
    /// operación no reciba algo que no puede procesar. Basta con el comienzo
    /// del archivo, así que funciona sin cargarlo entero en memoria.
    pub fn detectar(inicio: &[u8]) -> TipoArchivo {
        // Los PDF empiezan siempre por esta firma, por especificación.
        if inicio.starts_with(b"%PDF-") {
            return TipoArchivo::Pdf;
        }

        // Un byte cero descarta cualquier formato de texto de inmediato, y es
        // la señal más clara de que alguien renombró un binario.
        if inicio.contains(&0) {
            return TipoArchivo::Desconocido;
        }

        // `from_utf8` sobre un trozo cortado a la mitad puede fallar por un
        // carácter partido, así que probamos también sin los últimos bytes.
        let texto = std::str::from_utf8(inicio)
            .ok()
            .or_else(|| {
                let recorte = inicio.len().saturating_sub(4);
                std::str::from_utf8(&inicio[..recorte]).ok()
            })
            .map(str::trim_start);

        match texto {
            None => TipoArchivo::Desconocido,
            Some(texto) if texto.starts_with('{') || texto.starts_with('[') => TipoArchivo::Json,
            Some(texto) if texto.is_empty() => TipoArchivo::Desconocido,
            // Una sola línea sin separadores no es un CSV utilizable: le
            // faltaría el encabezado que nuestras operaciones necesitan.
            Some(texto) if texto.contains(',') || texto.contains(';') => TipoArchivo::Csv,
            Some(_) => TipoArchivo::Desconocido,
        }
    }

    pub fn etiqueta(&self) -> &'static str {
        match self {
            TipoArchivo::Csv => "csv",
            TipoArchivo::Pdf => "pdf",
            TipoArchivo::Json => "json",
            TipoArchivo::Desconocido => "desconocido",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoArchivo {
    /// Se entregó una URL de subida pero el cliente no ha confirmado.
    Pendiente,
    /// Los bytes están en el almacén y verificados.
    Disponible,
    /// Pasó su fecha de retención; los bytes ya no están.
    Vencido,
}

/// Cuánto tiempo se conserva un archivo antes de borrarse.
/// Sección 9 del PDF: "política clara de retención y eliminación".
pub const RETENCION_HORAS: i64 = 24;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Archivo {
    pub id: IdArchivo,
    pub organizacion: IdOrganizacion,

    /// El nombre que puso el cliente. Es solo para mostrar: NUNCA se usa para
    /// construir una ruta. Ahí es donde vive el path traversal.
    pub nombre_original: String,

    /// Dónde vive en el almacén. Se deriva del identificador, así que es
    /// imposible que contenga `../` o un nombre de dispositivo de Windows.
    pub clave: String,

    pub bytes: u64,
    pub tipo: TipoArchivo,
    pub estado: EstadoArchivo,

    /// Huella del contenido. Sirve para detectar subidas repetidas y para
    /// verificar integridad.
    pub sha256: Option<String>,

    pub creado_en: DateTime<Utc>,
    pub vence_en: DateTime<Utc>,
}

impl Archivo {
    /// Registra un archivo del que todavía no tenemos los bytes.
    pub fn pendiente(
        organizacion: IdOrganizacion,
        nombre_original: &str,
        ahora: DateTime<Utc>,
    ) -> Self {
        let id = IdArchivo::nuevo();
        Archivo {
            clave: Self::clave_de(&id),
            id,
            organizacion,
            nombre_original: nombre_para_mostrar(nombre_original),
            bytes: 0,
            tipo: TipoArchivo::Desconocido,
            estado: EstadoArchivo::Pendiente,
            sha256: None,
            creado_en: ahora,
            vence_en: ahora + Duration::hours(RETENCION_HORAS),
        }
    }

    /// La clave se construye solo con el identificador. Es la razón por la que
    /// el nombre del cliente no puede hacer daño.
    pub fn clave_de(id: &IdArchivo) -> String {
        format!("objetos/{}", id.uuid().simple())
    }

    /// Marca el archivo como disponible tras verificar sus bytes.
    pub fn confirmar(&mut self, bytes: u64, tipo: TipoArchivo, sha256: String) {
        self.bytes = bytes;
        self.tipo = tipo;
        self.sha256 = Some(sha256);
        self.estado = EstadoArchivo::Disponible;
    }

    pub fn esta_disponible(&self) -> bool {
        self.estado == EstadoArchivo::Disponible
    }

    pub fn ha_vencido(&self, ahora: DateTime<Utc>) -> bool {
        ahora >= self.vence_en
    }
}

/// Deja el nombre en algo seguro de mostrar y de registrar en logs.
///
/// Quita cualquier componente de ruta y los caracteres de control, que
/// permitirían falsificar líneas de log o romper una terminal.
fn nombre_para_mostrar(bruto: &str) -> String {
    let solo_nombre = bruto
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("archivo")
        .trim();

    let limpio: String = solo_nombre
        .chars()
        .filter(|c| !c.is_control())
        .take(200)
        .collect();

    if limpio.is_empty() || limpio == "." || limpio == ".." {
        "archivo".to_string()
    } else {
        limpio
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn un_csv_se_detecta_por_sus_separadores() {
        assert_eq!(
            TipoArchivo::detectar(b"nombre,email,telefono\nJuan,j@x.com,300"),
            TipoArchivo::Csv
        );
        assert_eq!(
            TipoArchivo::detectar(b"nombre;email\nJuan;j@x.com"),
            TipoArchivo::Csv
        );
    }

    #[test]
    fn un_pdf_se_detecta_por_su_firma() {
        assert_eq!(TipoArchivo::detectar(b"%PDF-1.7\n%..."), TipoArchivo::Pdf);
    }

    #[test]
    fn un_json_se_detecta_por_su_primer_caracter() {
        assert_eq!(TipoArchivo::detectar(b"  {\"a\":1}"), TipoArchivo::Json);
        assert_eq!(TipoArchivo::detectar(b"[{\"a\":1}]"), TipoArchivo::Json);
    }

    /// El caso que exige la sección 9 del PDF: un ejecutable renombrado a
    /// `.csv` no debe pasar la validación.
    #[test]
    fn un_binario_renombrado_a_csv_se_rechaza() {
        // Cabecera de un ejecutable de Windows, con sus bytes cero.
        let exe = b"MZ\x90\x00\x03\x00\x00\x00\x04\x00";
        assert_eq!(TipoArchivo::detectar(exe), TipoArchivo::Desconocido);
    }

    #[test]
    fn un_texto_sin_separadores_no_es_csv() {
        assert_eq!(
            TipoArchivo::detectar(b"esto es solo un parrafo de texto"),
            TipoArchivo::Desconocido
        );
    }

    #[test]
    fn un_archivo_vacio_es_desconocido() {
        assert_eq!(TipoArchivo::detectar(b""), TipoArchivo::Desconocido);
    }

    /// Un CSV con acentos cortado a la mitad de un carácter multibyte:
    /// no debe confundirse con un binario.
    #[test]
    fn un_csv_cortado_a_mitad_de_caracter_sigue_siendo_csv() {
        let completo = "nombre,ciudad\nJuan,Bogotá".as_bytes();
        let cortado = &completo[..completo.len() - 1];
        assert_eq!(TipoArchivo::detectar(cortado), TipoArchivo::Csv);
    }

    #[test]
    fn la_clave_del_almacen_no_depende_del_nombre_del_cliente() {
        let org = IdOrganizacion::nuevo();
        let archivo = Archivo::pendiente(org, "../../../etc/passwd", Utc::now());

        assert!(archivo.clave.starts_with("objetos/"));
        assert!(!archivo.clave.contains(".."));
        // El nombre se conserva para mostrar, pero sin los componentes de ruta.
        assert_eq!(archivo.nombre_original, "passwd");
    }

    #[test]
    fn los_caracteres_de_control_se_quitan_del_nombre() {
        let org = IdOrganizacion::nuevo();
        let archivo = Archivo::pendiente(org, "clientes\n[ERROR] falso.csv", Utc::now());
        assert_eq!(archivo.nombre_original, "clientes[ERROR] falso.csv");
    }

    #[test]
    fn un_nombre_vacio_no_deja_el_archivo_sin_nombre() {
        let org = IdOrganizacion::nuevo();
        let archivo = Archivo::pendiente(org, "   ", Utc::now());
        assert_eq!(archivo.nombre_original, "archivo");
    }

    #[test]
    fn un_archivo_nace_pendiente_y_se_confirma() {
        let org = IdOrganizacion::nuevo();
        let ahora = Utc::now();
        let mut archivo = Archivo::pendiente(org, "clientes.csv", ahora);

        assert!(!archivo.esta_disponible());
        archivo.confirmar(1024, TipoArchivo::Csv, "abc123".to_string());

        assert!(archivo.esta_disponible());
        assert_eq!(archivo.bytes, 1024);
        assert_eq!(archivo.tipo, TipoArchivo::Csv);
    }

    #[test]
    fn el_vencimiento_respeta_la_ventana_de_retencion() {
        let org = IdOrganizacion::nuevo();
        let ahora = Utc::now();
        let archivo = Archivo::pendiente(org, "x.csv", ahora);

        assert!(!archivo.ha_vencido(ahora));
        assert!(!archivo.ha_vencido(ahora + Duration::hours(RETENCION_HORAS - 1)));
        assert!(archivo.ha_vencido(ahora + Duration::hours(RETENCION_HORAS)));
    }
}
