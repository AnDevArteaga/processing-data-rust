use chrono::{DateTime, Utc};
use dp_core::Resumen;
use dp_dominio::{
    ApiKey, Archivo, ErrorDelJob, ErrorRepositorio, EstadoJob, Job, Movimiento, Organizacion,
};
use sqlx::FromRow;

#[derive(FromRow)]
pub struct ArchivoFila {
    pub id: String,
    pub organizacion: String,
    pub nombre_original: String,
    pub clave: String,
    pub bytes: i64,
    pub tipo: String,
    pub estado: String,
    pub sha256: Option<String>,
    pub creado_en: String,
    pub vence_en: String,
}

impl ArchivoFila {
    pub fn a_dominio(self) -> Result<Archivo, ErrorRepositorio> {
        Ok(Archivo {
            id: parsear(&self.id)?,
            organizacion: parsear(&self.organizacion)?,
            nombre_original: self.nombre_original,
            clave: self.clave,
            bytes: self.bytes as u64,
            tipo: json_o_texto(&self.tipo)?,
            estado: json_o_texto(&self.estado)?,
            sha256: self.sha256,
            creado_en: fecha(&self.creado_en)?,
            vence_en: fecha(&self.vence_en)?,
        })
    }
}

#[derive(FromRow)]
pub struct JobFila {
    pub id: String,
    pub organizacion: String,
    pub operacion: String,
    pub opciones: String,
    pub limites: String,
    pub entrada: String,
    pub salida: Option<String>,
    pub estado: String,
    pub progreso: i64,
    pub intentos: i64,
    pub max_intentos: i64,
    pub creditos_reservados: i64,
    pub creditos_cobrados: Option<i64>,
    pub resumen: Option<String>,
    pub error: Option<String>,
    pub creado_en: String,
    pub iniciado_en: Option<String>,
    pub terminado_en: Option<String>,
    pub reclamado_por: Option<String>,
    pub reclamado_hasta: Option<String>,
}

impl JobFila {
    pub fn a_dominio(self) -> Result<Job, ErrorRepositorio> {
        Ok(Job {
            id: parsear(&self.id)?,
            organizacion: parsear(&self.organizacion)?,
            operacion: self.operacion,
            opciones: json(&self.opciones)?,
            limites: json(&self.limites)?,
            entrada: parsear(&self.entrada)?,
            salida: self.salida.as_deref().map(parsear).transpose()?,
            estado: estado_job(&self.estado)?,
            progreso: self.progreso as u8,
            intentos: self.intentos as u32,
            max_intentos: self.max_intentos as u32,
            creditos_reservados: self.creditos_reservados as u64,
            creditos_cobrados: self.creditos_cobrados.map(|n| n as u64),
            resumen: self.resumen.as_deref().map(json::<Resumen>).transpose()?,
            error: self.error.as_deref().map(json::<ErrorDelJob>).transpose()?,
            creado_en: fecha(&self.creado_en)?,
            iniciado_en: self.iniciado_en.as_deref().map(fecha).transpose()?,
            terminado_en: self.terminado_en.as_deref().map(fecha).transpose()?,
            reclamado_por: self.reclamado_por,
            reclamado_hasta: self.reclamado_hasta.as_deref().map(fecha).transpose()?,
        })
    }
}

#[derive(FromRow)]
pub struct OrgFila {
    pub id: String,
    pub nombre: String,
    pub plan: String,
    pub creado_en: String,
}

impl OrgFila {
    pub fn a_dominio(self) -> Result<Organizacion, ErrorRepositorio> {
        Ok(Organizacion {
            id: parsear(&self.id)?,
            nombre: self.nombre,
            plan: json_o_texto(&self.plan)?,
            creado_en: fecha(&self.creado_en)?,
        })
    }
}

#[derive(FromRow)]
pub struct KeyFila {
    pub id: String,
    pub organizacion: String,
    pub nombre: String,
    pub prefijo: String,
    pub hash: String,
    pub revocada: i64,
    pub ultimo_uso: Option<String>,
    pub creada_en: String,
}

impl KeyFila {
    pub fn a_dominio(self) -> Result<ApiKey, ErrorRepositorio> {
        Ok(ApiKey {
            id: parsear(&self.id)?,
            organizacion: parsear(&self.organizacion)?,
            nombre: self.nombre,
            prefijo: self.prefijo,
            hash: self.hash,
            revocada: self.revocada != 0,
            ultimo_uso: self.ultimo_uso.as_deref().map(fecha).transpose()?,
            creada_en: fecha(&self.creada_en)?,
        })
    }
}

#[derive(FromRow)]
pub struct MovimientoFila {
    pub organizacion: String,
    pub tipo: String,
    pub cantidad: i64,
    pub job_id: Option<String>,
    pub descripcion: String,
    pub creado_en: String,
}

impl MovimientoFila {
    pub fn a_dominio(self) -> Result<Movimiento, ErrorRepositorio> {
        Ok(Movimiento {
            organizacion: parsear(&self.organizacion)?,
            tipo: json_o_texto(&self.tipo)?,
            cantidad: self.cantidad as u64,
            job: self.job_id.as_deref().map(parsear).transpose()?,
            descripcion: self.descripcion,
            creado_en: fecha(&self.creado_en)?,
        })
    }
}

fn parsear<T: std::str::FromStr>(texto: &str) -> Result<T, ErrorRepositorio>
where
    T::Err: std::fmt::Display,
{
    texto
        .parse()
        .map_err(|e| ErrorRepositorio::Interno(format!("id invalido '{texto}': {e}")))
}

fn fecha(texto: &str) -> Result<DateTime<Utc>, ErrorRepositorio> {
    DateTime::parse_from_rfc3339(texto)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| ErrorRepositorio::Interno(format!("fecha invalida '{texto}': {e}")))
}

fn json<T: serde::de::DeserializeOwned>(texto: &str) -> Result<T, ErrorRepositorio> {
    serde_json::from_str(texto).map_err(|e| ErrorRepositorio::Interno(e.to_string()))
}

/// Acepta tanto `"queued"` (JSON) como `queued` (texto plano).
fn json_o_texto<T: serde::de::DeserializeOwned>(texto: &str) -> Result<T, ErrorRepositorio> {
    if texto.starts_with('"') {
        json(texto)
    } else {
        json(&format!("\"{texto}\""))
    }
}

fn estado_job(texto: &str) -> Result<EstadoJob, ErrorRepositorio> {
    json_o_texto(texto)
}
