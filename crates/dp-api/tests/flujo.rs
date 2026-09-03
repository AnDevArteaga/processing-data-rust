//! Flujo HTTP de punta a punta: subir, encolar, procesar, consultar, descargar.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use dp_api::auth::AutenticadorFijo;
use dp_api::estado::Estado;
use dp_api::firma::Firmante;
use dp_api::{contexto_del_worker, enrutador};
use dp_dominio::{IdOrganizacion, Plan, RelojDelSistema};
use dp_memoria::{AlmacenLocal, RepositorioEnMemoria};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;

const TOKEN: &str = "token-de-prueba";

struct App {
    router: Router,
    estado: Arc<Estado>,
    raiz: std::path::PathBuf,
}

impl App {
    async fn nueva(plan: Plan) -> Self {
        let raiz = std::env::temp_dir().join(format!(
            "dp-api-flujo-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let repo = Arc::new(RepositorioEnMemoria::nuevo());
        let almacen = Arc::new(AlmacenLocal::nuevo(&raiz).await.unwrap());
        let org = IdOrganizacion::nuevo();
        let estado = Arc::new(Estado {
            archivos: repo.clone(),
            jobs: repo,
            almacen,
            autenticador: Arc::new(AutenticadorFijo::nuevo(TOKEN, org, plan)),
            reloj: Arc::new(RelojDelSistema),
            firmante: Arc::new(Firmante::nuevo("secreto-de-prueba")),
            base_publica: "http://test".to_string(),
        });
        App {
            router: enrutador(estado.clone()),
            estado,
            raiz,
        }
    }

    async fn pedir(&self, req: Request<Body>) -> (StatusCode, Value) {
        let respuesta = self.router.clone().oneshot(req).await.unwrap();
        let estado = respuesta.status();
        let bytes = respuesta.into_body().collect().await.unwrap().to_bytes();
        let cuerpo = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or(Value::String(String::from_utf8_lossy(&bytes).into_owned()))
        };
        (estado, cuerpo)
    }

    async fn get(&self, ruta: &str, token: Option<&str>) -> (StatusCode, Value) {
        let mut builder = Request::builder().uri(ruta).method("GET");
        if let Some(t) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {t}"));
        }
        self.pedir(builder.body(Body::empty()).unwrap()).await
    }

    async fn post(&self, ruta: &str, token: Option<&str>, cuerpo: Value) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .uri(ruta)
            .method("POST")
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(t) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {t}"));
        }
        self.pedir(builder.body(Body::from(cuerpo.to_string())).unwrap())
            .await
    }

    async fn put_bytes(&self, ruta: &str, bytes: Vec<u8>) -> (StatusCode, Value) {
        let req = Request::builder()
            .uri(ruta)
            .method("PUT")
            .body(Body::from(bytes))
            .unwrap();
        self.pedir(req).await
    }

    async fn get_bytes(&self, ruta: &str) -> (StatusCode, Vec<u8>) {
        let req = Request::builder()
            .uri(ruta)
            .method("GET")
            .body(Body::empty())
            .unwrap();
        let respuesta = self.router.clone().oneshot(req).await.unwrap();
        let estado = respuesta.status();
        let bytes = respuesta.into_body().collect().await.unwrap().to_bytes();
        (estado, bytes.to_vec())
    }

    async fn procesar(&self) {
        let ctx = contexto_del_worker(&self.estado);
        dp_worker::drenar(&ctx).await.unwrap();
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.raiz);
    }
}

fn csv_clientes() -> Vec<u8> {
    std::fs::read(format!(
        "{}/../../data/clientes.csv",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("data/clientes.csv")
}

fn csv_sin_columna() -> Vec<u8> {
    std::fs::read(format!(
        "{}/../../data/sin_columna.csv",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("data/sin_columna.csv")
}

fn ruta_relativa(url: &str) -> &str {
    url.find("/v1/").map(|i| &url[i..]).unwrap_or(url)
}

async fn subir(app: &App, nombre: &str, bytes: Vec<u8>) -> String {
    let (estado, cuerpo) = app
        .post(
            "/v1/files/presign",
            Some(TOKEN),
            json!({ "filename": nombre }),
        )
        .await;
    assert_eq!(estado, StatusCode::CREATED, "{cuerpo}");
    let file_id = cuerpo["file_id"].as_str().unwrap().to_string();
    let upload = ruta_relativa(cuerpo["upload_url"].as_str().unwrap()).to_string();

    let (estado, cuerpo) = app.put_bytes(&upload, bytes).await;
    assert_eq!(estado, StatusCode::OK, "{cuerpo}");
    assert_eq!(cuerpo["status"], "disponible");
    file_id
}

#[tokio::test]
async fn la_sonda_de_vida_no_pide_credencial() {
    let app = App::nueva(Plan::Pro).await;
    let (estado, cuerpo) = app.get("/health", None).await;
    assert_eq!(estado, StatusCode::OK);
    assert_eq!(cuerpo["ok"], true);
}

#[tokio::test]
async fn el_catalogo_publica_las_operaciones_del_motor() {
    let app = App::nueva(Plan::Pro).await;
    let (estado, cuerpo) = app.get("/v1/operations", None).await;
    assert_eq!(estado, StatusCode::OK);
    let nombres: Vec<&str> = cuerpo["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["name"].as_str().unwrap())
        .collect();
    assert!(nombres.contains(&"csv.clean"));
    assert!(nombres.contains(&"csv.inspect"));
}

#[tokio::test]
async fn sin_token_las_rutas_privadas_responden_401() {
    let app = App::nueva(Plan::Pro).await;
    let (estado, cuerpo) = app
        .post(
            "/v1/jobs",
            None,
            json!({ "operation": "csv.clean", "file_id": "file_0123456789abcdef0123456789abcdef" }),
        )
        .await;
    assert_eq!(estado, StatusCode::UNAUTHORIZED);
    assert_eq!(cuerpo["error"]["codigo"], "E_NO_AUTORIZADO");
}

#[tokio::test]
async fn el_flujo_completo_limpia_un_csv_y_deja_descargar_la_salida() {
    let app = App::nueva(Plan::Pro).await;
    let file_id = subir(&app, "clientes.csv", csv_clientes()).await;

    let (estado, cuerpo) = app
        .post(
            "/v1/jobs",
            Some(TOKEN),
            json!({ "operation": "csv.clean", "file_id": file_id }),
        )
        .await;
    assert_eq!(estado, StatusCode::ACCEPTED, "{cuerpo}");
    assert_eq!(cuerpo["status"], "queued");
    let job_id = cuerpo["job_id"].as_str().unwrap().to_string();

    app.procesar().await;

    let (estado, cuerpo) = app.get(&format!("/v1/jobs/{job_id}"), Some(TOKEN)).await;
    assert_eq!(estado, StatusCode::OK, "{cuerpo}");
    assert_eq!(cuerpo["status"], "completed");
    assert_eq!(cuerpo["result"]["leidas"], 15);
    assert_eq!(cuerpo["result"]["escritas"], 12);
    assert_eq!(cuerpo["result"]["duplicadas"], 3);
    assert_eq!(cuerpo["progress"], 100);
    assert!(cuerpo["output_file_id"].is_string());
    // El nombre del worker no puede filtrarse.
    let texto = cuerpo.to_string();
    assert!(!texto.contains("reclamado"));
    assert!(!texto.contains("worker-"));

    let salida = cuerpo["output_file_id"].as_str().unwrap();
    let (estado, cuerpo) = app
        .get(&format!("/v1/files/{salida}/download"), Some(TOKEN))
        .await;
    assert_eq!(estado, StatusCode::OK, "{cuerpo}");
    let download = ruta_relativa(cuerpo["download_url"].as_str().unwrap());

    let (estado, bytes) = app.get_bytes(download).await;
    assert_eq!(estado, StatusCode::OK);
    let texto = String::from_utf8(bytes).unwrap();
    assert!(texto.starts_with("nombre,email,telefono,ciudad,email_estado,telefono_valido"));
    assert!(texto.contains("juan@gmail.com"));
}

#[tokio::test]
async fn un_csv_invalido_queda_en_failed_con_el_codigo_del_motor() {
    let app = App::nueva(Plan::Pro).await;
    let file_id = subir(&app, "roto.csv", csv_sin_columna()).await;

    let (estado, cuerpo) = app
        .post(
            "/v1/jobs",
            Some(TOKEN),
            json!({ "operation": "csv.clean", "file_id": file_id }),
        )
        .await;
    assert_eq!(estado, StatusCode::ACCEPTED, "{cuerpo}");
    let job_id = cuerpo["job_id"].as_str().unwrap().to_string();

    app.procesar().await;

    let (estado, cuerpo) = app.get(&format!("/v1/jobs/{job_id}"), Some(TOKEN)).await;
    assert_eq!(estado, StatusCode::OK);
    assert_eq!(cuerpo["status"], "failed");
    assert_eq!(cuerpo["error"]["code"], "E_COLUMNA_FALTANTE");
    assert_eq!(cuerpo["error"]["client_error"], true);
}

#[tokio::test]
async fn se_puede_cancelar_un_job_encolado_y_no_uno_inexistente() {
    let app = App::nueva(Plan::Pro).await;
    let file_id = subir(&app, "clientes.csv", csv_clientes()).await;

    let (_, cuerpo) = app
        .post(
            "/v1/jobs",
            Some(TOKEN),
            json!({ "operation": "csv.inspect", "file_id": file_id }),
        )
        .await;
    let job_id = cuerpo["job_id"].as_str().unwrap().to_string();

    let (estado, cuerpo) = app
        .post(&format!("/v1/jobs/{job_id}/cancel"), Some(TOKEN), json!({}))
        .await;
    assert_eq!(estado, StatusCode::OK, "{cuerpo}");
    assert_eq!(cuerpo["status"], "cancelled");

    let (estado, cuerpo) = app
        .post(
            "/v1/jobs/job_0123456789abcdef0123456789abcdef/cancel",
            Some(TOKEN),
            json!({}),
        )
        .await;
    assert_eq!(estado, StatusCode::NOT_FOUND);
    assert_eq!(cuerpo["error"]["codigo"], "E_JOB_NO_ENCONTRADO");
}

#[tokio::test]
async fn el_plan_gratuito_no_puede_tener_dos_jobs_en_vuelo() {
    let app = App::nueva(Plan::Free).await;
    let file_id = subir(&app, "clientes.csv", csv_clientes()).await;

    let (estado, _) = app
        .post(
            "/v1/jobs",
            Some(TOKEN),
            json!({ "operation": "csv.inspect", "file_id": file_id }),
        )
        .await;
    assert_eq!(estado, StatusCode::ACCEPTED);

    let (estado, cuerpo) = app
        .post(
            "/v1/jobs",
            Some(TOKEN),
            json!({ "operation": "csv.inspect", "file_id": file_id }),
        )
        .await;
    assert_eq!(estado, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(cuerpo["error"]["codigo"], "E_DEMASIADOS_JOBS");
}

#[tokio::test]
async fn un_json_no_sirve_para_una_operacion_de_csv() {
    let app = App::nueva(Plan::Pro).await;
    let file_id = subir(&app, "datos.json", b"{\"a\":1}".to_vec()).await;

    let (estado, cuerpo) = app
        .post(
            "/v1/jobs",
            Some(TOKEN),
            json!({ "operation": "csv.clean", "file_id": file_id }),
        )
        .await;
    assert_eq!(estado, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(cuerpo["error"]["codigo"], "E_TIPO_NO_SOPORTADO");
}

#[tokio::test]
async fn una_operacion_inventada_se_rechaza_antes_de_tocar_el_archivo() {
    let app = App::nueva(Plan::Pro).await;
    let (estado, cuerpo) = app
        .post(
            "/v1/jobs",
            Some(TOKEN),
            json!({
                "operation": "pdf.ocr",
                "file_id": "file_0123456789abcdef0123456789abcdef"
            }),
        )
        .await;
    assert_eq!(estado, StatusCode::BAD_REQUEST);
    assert_eq!(cuerpo["error"]["codigo"], "E_OPERACION_DESCONOCIDA");
}

#[tokio::test]
async fn un_archivo_ajeno_parece_inexistente() {
    let app = App::nueva(Plan::Pro).await;
    let (estado, cuerpo) = app
        .get(
            "/v1/files/file_0123456789abcdef0123456789abcdef",
            Some(TOKEN),
        )
        .await;
    assert_eq!(estado, StatusCode::NOT_FOUND);
    assert_eq!(cuerpo["error"]["codigo"], "E_ARCHIVO_NO_ENCONTRADO");
}

#[tokio::test]
async fn el_listado_solo_muestra_los_jobs_de_esta_organizacion() {
    let app = App::nueva(Plan::Pro).await;
    let file_id = subir(&app, "clientes.csv", csv_clientes()).await;
    app.post(
        "/v1/jobs",
        Some(TOKEN),
        json!({ "operation": "csv.inspect", "file_id": file_id }),
    )
    .await;

    let (estado, cuerpo) = app.get("/v1/jobs", Some(TOKEN)).await;
    assert_eq!(estado, StatusCode::OK);
    assert_eq!(cuerpo["count"], 1);
    assert_eq!(cuerpo["data"][0]["status"], "queued");
}
