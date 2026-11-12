//! Lớp HTTP chung: route dưới `/api/v1/<module>`, request id, lỗi luôn là Problem
//! Details, giới hạn body, timeout, CORS, graceful shutdown.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
// Clippy chỉ tự miễn cho hàm `#[test]`, không miễn cho hàm phụ, nên khai ở đây.
// Trong `src/` thì KHÔNG được khai dòng này (`cargo xtask check` chặn).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use autoxide_kernel::config::{
    ConfigReader, ConfigSource, CorsOrigins, HttpConfig, LogConfig, LogFormat, LogLevel,
};
use autoxide_kernel::error::Error;
use autoxide_kernel::http::{self, Api, Shutdown, ValidJson};
use autoxide_kernel::response::{Data, Problem};
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{Path, Query};
use axum::http::{Request, StatusCode, header};
use axum::response::Response;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

#[derive(Debug, Deserialize, Validate, ToSchema)]
struct CreateNote {
    #[validate(length(min = 1, max = 20))]
    title: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
struct Note {
    id: u32,
    title: String,
}

/// List notes.
#[utoipa::path(get, path = "/", responses((status = 200, description = "The notes.", body = Data<Vec<Note>>)))]
async fn list_notes() -> Data<Vec<Note>> {
    Data { data: vec![Note { id: 1, title: "first".to_owned() }] }
}

/// Create a note.
#[utoipa::path(post, path = "/", request_body = CreateNote, responses((status = 201, description = "The created note.", body = Data<Note>)))]
async fn create_note(ValidJson(input): ValidJson<CreateNote>) -> (StatusCode, Data<Note>) {
    (StatusCode::CREATED, Data { data: Note { id: 2, title: input.title } })
}

/// Get one note.
#[utoipa::path(get, path = "/{id}", responses(
        (status = 200, description = "The note.", body = Data<Note>),
        (status = 404, description = "No note has this id.", body = Problem),
    ))]
async fn get_note(Path(id): Path<u32>) -> Result<Data<Note>, Error> {
    Err(Error::not_found("notes.note_not_found", "Note {id} does not exist.").with_param("id", id))
}

#[derive(Deserialize)]
struct Paging {
    #[allow(dead_code)]
    size: u32,
}

async fn typed_query(Query(_): Query<Paging>) -> &'static str {
    "ok"
}

async fn slow() -> &'static str {
    tokio::time::sleep(Duration::from_millis(300)).await;
    "done"
}

async fn panics() -> &'static str {
    panic!("secret internal detail");
}

async fn echo(body: Bytes) -> Bytes {
    body
}

fn notes_routes() -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_notes, create_note))
        .routes(routes!(get_note))
        .route("/typed", axum::routing::get(typed_query))
        .route("/slow", axum::routing::get(slow))
        .route("/panic", axum::routing::get(panics))
        .route("/echo", axum::routing::post(echo))
}

fn default_config() -> HttpConfig {
    let source = ConfigSource::default();
    let mut reader = ConfigReader::new(&source);
    let config = HttpConfig::read(&mut reader);
    reader.finish(config).unwrap().0
}

fn api_with(config: &HttpConfig) -> Api {
    http::build(
        config,
        OpenApiRouter::new(),
        vec![("notes", notes_routes()), ("empty", OpenApiRouter::new())],
        http::Extras::default(),
    )
}

fn app() -> Router {
    api_with(&default_config()).router
}

fn get(path: &str) -> Request<Body> {
    Request::builder().uri(path).body(Body::empty()).unwrap()
}

fn post_json(path: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap()
}

async fn send(router: Router, request: Request<Body>) -> Response {
    router.oneshot(request).await.unwrap()
}

async fn json(response: Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// Kiểm response là Problem Details đúng dạng và trả về thân của nó.
async fn problem(response: Response, status: u16, code: &str) -> Problem {
    assert_eq!(response.status().as_u16(), status);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/problem+json");
    let problem: Problem = serde_json::from_value(json(response).await).unwrap();
    assert_eq!((problem.status, problem.code.as_str()), (status, code), "{problem:?}");
    assert_eq!(problem.type_uri, "about:blank");
    problem
}

#[tokio::test]
async fn route_cua_module_nam_duoi_api_v1_ten_module() {
    let response = send(app(), get("/api/v1/notes")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json(response).await, serde_json::json!({"data": [{"id": 1, "title": "first"}]}));

    // Tài liệu OpenAPI ghi đúng đường dẫn mà router phục vụ.
    let api = api_with(&default_config());
    let paths: Vec<&str> = api.openapi.paths.paths.keys().map(String::as_str).collect();
    assert_eq!(paths, ["/api/v1/notes", "/api/v1/notes/{id}"]);
    assert_eq!(api.openapi.info.title, "Autoxide API");
}

#[tokio::test]
async fn loi_cua_handler_la_problem_details() {
    let response = send(app(), get("/api/v1/notes/7")).await;

    let problem = problem(response, 404, "notes.note_not_found").await;
    assert_eq!(problem.detail, "Note 7 does not exist.");
    assert_eq!(problem.title, "Not Found");
}

#[tokio::test]
async fn khong_co_route_va_sai_method_cung_la_problem_details() {
    for path in ["/api/v1/nope", "/api/v1/notes/1/extra", "/nowhere", "/"] {
        let found = problem(send(app(), get(path)).await, 404, "not_found").await;
        assert!(found.detail.contains("No such endpoint"), "{path}: {found:?}");
    }

    let request = Request::builder().method("DELETE").uri("/api/v1/notes").body(Body::empty());
    let response = send(app(), request.unwrap()).await;
    // Header `allow` của lỗi 405 được giữ: client biết method nào dùng được.
    let allow = response.headers()[header::ALLOW].to_str().unwrap().to_owned();
    assert!(allow.contains("GET") && allow.contains("POST"), "{allow}");
    problem(response, 405, "method_not_allowed").await;
}

#[tokio::test]
async fn tham_so_query_sai_kieu_la_problem_details_co_mo_ta() {
    let response = send(app(), get("/api/v1/notes/typed?size=big")).await;

    let problem = problem(response, 400, "bad_request").await;
    // Câu mô tả của thư viện được giữ lại để client biết sửa gì.
    assert!(problem.detail.contains("size"), "{problem:?}");
}

#[tokio::test]
async fn request_id_duoc_sinh_va_tra_lai() {
    let response = send(app(), get("/api/v1/notes")).await;
    let generated = response.headers()["x-request-id"].to_str().unwrap();
    assert_eq!(Uuid::parse_str(generated).unwrap().get_version_num(), 7);

    // Response lỗi cũng có request id.
    let response = send(app(), get("/nowhere")).await;
    assert!(response.headers().contains_key("x-request-id"));
}

#[tokio::test]
async fn request_id_cua_client_duoc_giu_neu_hop_le() {
    let with_id = |id: &str| {
        Request::builder()
            .uri("/api/v1/notes")
            .header("x-request-id", id)
            .body(Body::empty())
            .unwrap()
    };

    let response = send(app(), with_id("trace-2026.11:abc_DEF")).await;
    assert_eq!(response.headers()["x-request-id"], "trace-2026.11:abc_DEF");

    // ID có ký tự lạ hoặc quá dài bị thay bằng ID mới: thứ này đi vào log.
    for untrusted in ["has space", "<script>", &"a".repeat(129)] {
        let response = send(app(), with_id(untrusted)).await;
        let returned = response.headers()["x-request-id"].to_str().unwrap();
        assert!(Uuid::parse_str(returned).is_ok(), "{untrusted} -> {returned}");
    }
}

#[tokio::test]
async fn valid_json_nhan_dau_vao_hop_le() {
    let response = send(app(), post_json("/api/v1/notes", r#"{"title": "hello"}"#)).await;

    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(json(response).await, serde_json::json!({"data": {"id": 2, "title": "hello"}}));
}

#[tokio::test]
async fn valid_json_tu_choi_dau_vao_sai_voi_ma_loi_rieng() {
    // JSON hỏng.
    let response = send(app(), post_json("/api/v1/notes", r#"{"title": "#)).await;
    problem(response, 400, "invalid_json").await;

    // JSON đúng nhưng thiếu trường: mô tả nêu tên trường.
    let response = send(app(), post_json("/api/v1/notes", r#"{}"#)).await;
    let missing = problem(response, 422, "invalid_body").await;
    assert!(missing.detail.contains("title"), "{missing:?}");

    // Thiếu content-type.
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/notes")
        .body(Body::from(r#"{"title": "hello"}"#))
        .unwrap();
    problem(send(app(), request).await, 415, "unsupported_media_type").await;

    // Đọc được nhưng giá trị không qua kiểm tra: lỗi theo từng trường.
    let response = send(app(), post_json("/api/v1/notes", r#"{"title": ""}"#)).await;
    let invalid = problem(response, 422, "validation_failed").await;
    assert_eq!(invalid.errors.len(), 1);
    assert_eq!(
        (invalid.errors[0].field.as_str(), invalid.errors[0].code.as_str()),
        ("title", "length")
    );
    assert_eq!(invalid.errors[0].detail, "must be between 1 and 20 characters long");
}

#[tokio::test]
async fn body_qua_lon_bi_tu_choi() {
    let mut config = default_config();
    config.body_limit = 1024;
    let large = "x".repeat(2000);

    // Có `content-length`: bị chặn trước khi đọc thân.
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/notes/echo")
        .header(header::CONTENT_LENGTH, large.len())
        .body(Body::from(large.clone()))
        .unwrap();
    problem(send(api_with(&config).router, request).await, 413, "payload_too_large").await;

    // Không có `content-length` (gửi theo từng đoạn): bị chặn trong lúc đọc.
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/notes/echo")
        .body(Body::from(large))
        .unwrap();
    problem(send(api_with(&config).router, request).await, 413, "payload_too_large").await;

    // Thân trong giới hạn thì qua.
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/notes/echo")
        .body(Body::from("small"))
        .unwrap();
    assert_eq!(send(api_with(&config).router, request).await.status(), StatusCode::OK);
}

/// Route được module khai giới hạn riêng nhận thân lớn hơn giới hạn chung; route khác
/// thì không.
#[tokio::test]
async fn route_khai_gioi_han_rieng() {
    let mut config = default_config();
    config.body_limit = 1024;
    let extras = http::Extras {
        body_limits: vec![("notes", http::BodyLimit::new(axum::http::Method::POST, "/echo", 4096))],
        ..Default::default()
    };
    let api = http::build(&config, OpenApiRouter::new(), vec![("notes", notes_routes())], extras);
    let echo = |body: String| {
        Request::builder()
            .method("POST")
            .uri("/api/v1/notes/echo")
            .header(header::CONTENT_LENGTH, body.len())
            .body(Body::from(body))
            .unwrap()
    };

    assert_eq!(send(api.router.clone(), echo("x".repeat(2000))).await.status(), StatusCode::OK);
    let response = send(api.router.clone(), echo("x".repeat(5000))).await;
    let rejected = problem(response, 413, "payload_too_large").await;
    assert!(rejected.detail.contains("4096 bytes"), "{}", rejected.detail);
    let other = send(
        api.router,
        post_json("/api/v1/notes", &format!(r#"{{"title": "{}"}}"#, "x".repeat(2000))),
    );
    problem(other.await, 413, "payload_too_large").await;
}

#[tokio::test]
async fn request_chay_qua_lau_bi_cat() {
    let mut config = default_config();
    config.request_timeout = Duration::from_millis(50);
    let started = Instant::now();

    let response = send(api_with(&config).router, get("/api/v1/notes/slow")).await;

    assert!(started.elapsed() < Duration::from_millis(250), "{:?}", started.elapsed());
    let problem = problem(response, 503, "request_timeout").await;
    assert!(problem.detail.contains("did not finish handling the request"), "{problem:?}");

    // Timeout đủ dài thì request chạy xong bình thường.
    assert_eq!(send(app(), get("/api/v1/notes/slow")).await.status(), StatusCode::OK);
}

#[tokio::test]
async fn handler_panic_thanh_loi_500_khong_lo_chi_tiet() {
    let response = send(app(), get("/api/v1/notes/panic")).await;

    let problem = problem(response, 500, "internal_error").await;
    assert!(!problem.detail.contains("secret internal detail"), "{problem:?}");
}

#[tokio::test]
async fn cors_chi_cho_origin_da_cau_hinh() {
    let preflight = |origin: &str| {
        Request::builder()
            .method("OPTIONS")
            .uri("/api/v1/notes")
            .header(header::ORIGIN, origin)
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
            .body(Body::empty())
            .unwrap()
    };
    let allow_origin = |response: &Response| {
        response
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .map(|value| value.to_str().unwrap().to_owned())
    };

    // Mặc định: không origin nào được phép.
    let response = send(app(), preflight("https://app.example.com")).await;
    assert_eq!(allow_origin(&response), None);

    let mut config = default_config();
    config.cors_origins = CorsOrigins::List(vec!["https://app.example.com".to_owned()]);
    let response = send(api_with(&config).router, preflight("https://app.example.com")).await;
    assert_eq!(allow_origin(&response).as_deref(), Some("https://app.example.com"));
    let response = send(api_with(&config).router, preflight("https://evil.example.com")).await;
    assert_eq!(allow_origin(&response), None);

    config.cors_origins = CorsOrigins::Any;
    let response = send(api_with(&config).router, preflight("https://anything.example.com")).await;
    assert_eq!(allow_origin(&response).as_deref(), Some("*"));

    // Trình duyệt gửi được `If-Match` và đọc được `ETag`, `Location`.
    let allowed = response.headers()[header::ACCESS_CONTROL_ALLOW_HEADERS].to_str().unwrap();
    assert!(allowed.contains("if-match"), "{allowed}");
    let request = Request::builder()
        .uri("/api/v1/notes")
        .header(header::ORIGIN, "https://anything.example.com")
        .body(Body::empty())
        .unwrap();
    let response = send(api_with(&config).router, request).await;
    let exposed = response.headers()[header::ACCESS_CONTROL_EXPOSE_HEADERS].to_str().unwrap();
    for name in ["etag", "location", "retry-after", "x-request-id"] {
        assert!(exposed.contains(name), "{name} not in {exposed}");
    }
}

/// Gửi một request GET qua kết nối TCP thật; trả về toàn bộ response dạng chữ.
async fn raw_get(address: std::net::SocketAddr, path: &str) -> std::io::Result<String> {
    let mut stream = tokio::net::TcpStream::connect(address).await?;
    let request = format!("GET {path} HTTP/1.1\r\nhost: test\r\nconnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await?;
    let mut response = String::new();
    stream.read_to_string(&mut response).await?;
    Ok(response)
}

#[tokio::test]
async fn tat_may_chu_cho_request_dang_chay_xong() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(http::serve(
        listener,
        app(),
        async {
            let _ = stopped.await;
        },
        Duration::from_secs(5),
    ));

    // Một request chậm (300 ms) đang chạy thì có lệnh tắt.
    let in_flight = tokio::spawn(raw_get(address, "/api/v1/notes/slow"));
    tokio::time::sleep(Duration::from_millis(100)).await;
    stop.send(()).unwrap();

    // Request đang dở vẫn nhận đủ response…
    let response = in_flight.await.unwrap().unwrap();
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(response.ends_with("done"), "{response}");
    // …máy chủ dừng êm, và không nhận kết nối mới nữa.
    assert_eq!(server.await.unwrap().unwrap(), Shutdown::Graceful);
    assert!(raw_get(address, "/api/v1/notes").await.is_err());
}

#[tokio::test]
async fn tat_may_chu_khong_cho_qua_thoi_gian_cho_phep() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(http::serve(
        listener,
        app(),
        async {
            let _ = stopped.await;
        },
        // Request chậm cần 300 ms; chỉ cho chờ 50 ms.
        Duration::from_millis(50),
    ));

    let in_flight = tokio::spawn(raw_get(address, "/api/v1/notes/slow"));
    tokio::time::sleep(Duration::from_millis(100)).await;
    let started = Instant::now();
    stop.send(()).unwrap();

    assert_eq!(server.await.unwrap().unwrap(), Shutdown::TimedOut);
    assert!(started.elapsed() < Duration::from_millis(250), "{:?}", started.elapsed());
    drop(in_flight);
}

#[tokio::test]
async fn tai_lieu_openapi_duoc_phuc_vu_va_khop_voi_router() {
    let api = api_with(&default_config());

    let response = send(api.router, get("/api/v1/openapi.json")).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
    let served = json(response).await;
    assert_eq!(served, serde_json::to_value(&api.openapi).unwrap());
    assert_eq!(served["info"]["title"], "Autoxide API");
    assert!(served["info"]["description"].as_str().unwrap().contains("RFC 9457"));
}

#[tokio::test]
async fn tai_lieu_tu_gan_tag_ma_dinh_danh_va_loi_mac_dinh_cho_moi_endpoint() {
    let document = serde_json::to_value(api_with(&default_config()).openapi).unwrap();

    let list = &document["paths"]["/api/v1/notes"]["get"];
    // Tag là tên module; operationId mang tiền tố module nên không trùng giữa các module.
    assert_eq!(list["tags"], serde_json::json!(["notes"]));
    assert_eq!(list["operationId"], "notes_list_notes");
    assert_eq!(list["summary"], "List notes.");
    // Endpoint nào cũng có response `default` trỏ tới Problem Details, dù module
    // không tự khai.
    assert_eq!(
        list["responses"]["default"]["content"]["application/problem+json"]["schema"]["$ref"],
        "#/components/schemas/Problem"
    );
    let schemas = document["components"]["schemas"].as_object().unwrap();
    assert!(schemas.contains_key("Problem") && schemas.contains_key("ProblemField"));
    // Response lỗi module tự khai thì được giữ nguyên.
    let get_one = &document["paths"]["/api/v1/notes/{id}"]["get"];
    assert!(get_one["responses"]["404"].is_object());
    assert_eq!(get_one["operationId"], "notes_get_note");
}

#[tokio::test]
async fn trang_xem_tai_lieu_chi_co_khi_duoc_bat() {
    // Mặc định tắt.
    problem(send(app(), get("/api/docs")).await, 404, "not_found").await;

    let mut config = default_config();
    config.api_docs = true;
    let response = send(api_with(&config).router, get("/api/docs")).await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let html = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(html.contains("/api/v1/notes"), "docs page does not embed the OpenAPI document");
}

// Handler không có doc comment và không khai response thành công.
#[utoipa::path(get, path = "/undocumented")]
async fn undocumented() {}

// Hai handler ở hai nơi nhưng cùng tên hàm sau khi ghép sẽ trùng operationId.
mod duplicate {
    /// Same name as another operation.
    #[utoipa::path(get, path = "/duplicate", operation_id = "list_notes", responses((status = 200, description = "ok")))]
    pub async fn duplicate() {}
}

#[tokio::test]
async fn bo_kiem_tai_lieu_bat_endpoint_thieu_mo_ta() {
    // Tài liệu của các route mẫu đạt.
    assert_eq!(http::lint(&api_with(&default_config()).openapi), Vec::<String>::new());

    let routes = notes_routes().routes(routes!(undocumented)).routes(routes!(duplicate::duplicate));
    let api = http::build(
        &default_config(),
        OpenApiRouter::new(),
        vec![("notes", routes)],
        http::Extras::default(),
    );

    assert_eq!(
        http::lint(&api.openapi),
        [
            "GET /api/v1/notes/duplicate: operationId `notes_list_notes` is used twice",
            "GET /api/v1/notes/undocumented: no summary — add a `///` doc comment to the handler",
            "GET /api/v1/notes/undocumented: no success response — declare what the endpoint returns",
        ]
    );
}

/// Nơi ghi log vào bộ nhớ, để test đọc lại.
#[derive(Clone, Default)]
struct LogBuffer(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for LogBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogBuffer {
    type Writer = LogBuffer;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Gửi `request` với log JSON ở mức `level` được bắt lại; trả về các dòng log.
async fn logged(level: LogLevel, request: Request<Body>) -> Vec<serde_json::Value> {
    let buffer = LogBuffer::default();
    let config = LogConfig { level, format: LogFormat::Json };
    let subscriber = autoxide_kernel::logging::subscriber(&config, buffer.clone(), Vec::new());
    // Bộ nhận log này chỉ có hiệu lực trên thread hiện tại, tới khi `_guard` bị drop.
    let _guard = tracing::subscriber::set_default(subscriber);
    send(app(), request).await;
    let text = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
    text.lines().map(|line| serde_json::from_str(line).unwrap()).collect()
}

#[tokio::test]
async fn moi_request_mot_dong_log_kem_request_id() {
    let request = Request::builder()
        .uri("/api/v1/notes/7?token=s3cr3t")
        .header("x-request-id", "trace-1")
        .body(Body::empty())
        .unwrap();

    let lines = logged(LogLevel::Info, request).await;

    assert_eq!(lines.len(), 1, "{lines:?}");
    let line = &lines[0];
    assert_eq!(line["message"], "request completed");
    assert_eq!(line["level"], "INFO");
    assert_eq!(line["status"], 404);
    assert_eq!(line["route"], "/api/v1/notes/{id}");
    assert_eq!(line["span"]["request_id"], "trace-1");
    assert_eq!(line["span"]["method"], "GET");
    assert_eq!(line["span"]["path"], "/api/v1/notes/7");
    assert!(line["elapsed_ms"].is_u64(), "{line}");
    // Query string không bao giờ vào log.
    assert!(!line.to_string().contains("s3cr3t"), "{line}");
}

#[tokio::test]
async fn loi_5xx_duoc_ghi_kem_nguyen_nhan_goc() {
    let lines = logged(LogLevel::Info, get("/api/v1/notes/panic")).await;

    let line = lines.last().unwrap();
    assert_eq!(line["level"], "ERROR");
    assert_eq!(line["message"], "request failed");
    assert_eq!(line["status"], 500);
    // Thứ client không được thấy thì người vận hành thấy trong log.
    assert!(line["error"].as_str().unwrap().contains("secret internal detail"), "{line}");
}

#[tokio::test]
async fn request_khong_khop_route_va_health_check_duoc_ghi_dung_muc() {
    let lines = logged(LogLevel::Info, get("/nowhere")).await;
    assert_eq!(lines[0]["route"], "unmatched");
    assert_eq!(lines[0]["status"], 404);

    // Health check ghi ở mức debug: ở mức info thì không có dòng nào.
    assert!(logged(LogLevel::Info, get("/health/live")).await.is_empty());
    let lines = logged(LogLevel::Debug, get("/health/live")).await;
    assert!(lines.iter().any(|line| line["span"]["path"] == "/health/live"), "{lines:?}");
}

/// Request GET `path` đến từ địa chỉ `client` (như khi đi qua socket thật).
fn from_client(path: &str, client: [u8; 4]) -> Request<Body> {
    let mut request = Request::builder().uri(path).body(Body::empty()).unwrap();
    let address = std::net::SocketAddr::from((client, 40000));
    request.extensions_mut().insert(axum::extract::ConnectInfo(address));
    request
}

#[tokio::test]
async fn qua_gioi_han_thi_429_kem_retry_after() {
    let mut config = default_config();
    config.rate_limit.per_minute = 60;
    config.rate_limit.burst = 2;
    let app = api_with(&config).router;

    for _ in 0..2 {
        let response = app.clone().oneshot(from_client("/api/v1/notes", [192, 0, 2, 1])).await;
        assert_eq!(response.unwrap().status(), StatusCode::OK);
    }
    let limited = app.clone().oneshot(from_client("/api/v1/notes", [192, 0, 2, 1])).await.unwrap();

    assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(limited.headers()[header::RETRY_AFTER], "1");
    assert!(limited.headers().contains_key("x-request-id"));
    problem(limited, 429, "rate_limited").await;
    // Client khác vẫn được phục vụ; health check không bị đếm.
    let other = app.clone().oneshot(from_client("/api/v1/notes", [192, 0, 2, 2])).await;
    assert_eq!(other.unwrap().status(), StatusCode::OK);
    let health = app.clone().oneshot(from_client("/health/live", [192, 0, 2, 1])).await;
    assert_ne!(health.unwrap().status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn tat_rate_limiting_thi_khong_gioi_han() {
    let mut config = default_config();
    config.rate_limit.per_minute = 0;
    config.rate_limit.burst = 1;
    let app = api_with(&config).router;

    for _ in 0..5 {
        let response = app.clone().oneshot(from_client("/api/v1/notes", [192, 0, 2, 1])).await;
        assert_eq!(response.unwrap().status(), StatusCode::OK);
    }
}

fn localized_app() -> Router {
    let extras = http::Extras {
        translations: autoxide_kernel::i18n::Translations::new(vec![(
            "notes",
            vec![("vi", r#"{"notes.note_not_found": "Không có ghi chú {id}."}"#)],
        )])
        .unwrap(),
        ..Default::default()
    };
    http::build(&default_config(), OpenApiRouter::new(), vec![("notes", notes_routes())], extras)
        .router
}

fn with_language(path: &str, language: Option<&str>) -> Request<Body> {
    let mut request = Request::builder().uri(path);
    if let Some(language) = language {
        request = request.header(header::ACCEPT_LANGUAGE, language);
    }
    request.body(Body::empty()).unwrap()
}

#[tokio::test]
async fn loi_duoc_dich_theo_accept_language() {
    let app = localized_app();

    let english = send(app.clone(), with_language("/api/v1/notes/7", None)).await;
    assert_eq!(english.headers()[header::CONTENT_LANGUAGE], "en");
    let problem_en = problem(english, 404, "notes.note_not_found").await;
    assert_eq!(problem_en.detail, "Note 7 does not exist.");

    let vietnamese =
        send(app.clone(), with_language("/api/v1/notes/7", Some("vi-VN,vi;q=0.9,en;q=0.5"))).await;
    assert_eq!(vietnamese.headers()[header::CONTENT_LANGUAGE], "vi");
    let problem_vi = problem(vietnamese, 404, "notes.note_not_found").await;
    assert_eq!(problem_vi.detail, "Không có ghi chú 7.");
    // `code` không đổi theo ngôn ngữ: máy vẫn rẽ nhánh như cũ.
    assert_eq!(problem_vi.code, problem_en.code);

    // Lỗi do thư viện sinh ra (không có route) cũng được dịch, bằng bản dịch của kernel.
    let missing = send(app.clone(), with_language("/api/v1/nope", Some("vi"))).await;
    let problem_missing = problem(missing, 404, "not_found").await;
    assert!(problem_missing.detail.starts_with("Không có endpoint này"), "{problem_missing:?}");

    // Ngôn ngữ chưa hỗ trợ: tiếng Anh.
    let french = send(app, with_language("/api/v1/notes/7", Some("fr"))).await;
    assert_eq!(french.headers()[header::CONTENT_LANGUAGE], "en");
}

#[tokio::test]
async fn endpoint_deprecated_co_header_va_danh_dau_trong_tai_lieu() {
    let deprecation = http::Deprecation::new(axum::http::Method::GET, "/{id}", "2026-11-01")
        .sunset("2027-05-01")
        .successor("/api/v2/notes/{id}");
    let extras = http::Extras { deprecations: vec![("notes", deprecation)], ..Default::default() };
    let api = http::build(
        &default_config(),
        OpenApiRouter::new(),
        vec![("notes", notes_routes())],
        extras,
    );

    let old = send(api.router.clone(), get("/api/v1/notes/7")).await;
    assert_eq!(old.headers()["deprecation"], "@1793491200");
    assert_eq!(old.headers()["sunset"], "Sat, 01 May 2027 00:00:00 GMT");
    assert_eq!(old.headers()[header::LINK], "</api/v2/notes/{id}>; rel=\"successor-version\"");
    let current = send(api.router.clone(), get("/api/v1/notes")).await;
    assert!(!current.headers().contains_key("deprecation"));

    let document = serde_json::to_value(&api.openapi).unwrap();
    let operation = &document["paths"]["/api/v1/notes/{id}"]["get"];
    assert_eq!(operation["deprecated"], true);
    let description = operation["description"].as_str().unwrap();
    assert!(description.contains("Deprecated** since 2026-11-01"), "{description}");
    assert!(description.contains("removed on 2027-05-01"), "{description}");
    assert!(document["paths"]["/api/v1/notes"]["get"].get("deprecated").is_none());
}

/// Header chung được khai trên từng operation để SDK sinh tự động có sẵn tham số:
/// `Accept-Language` ở mọi nơi; `Idempotency-Key` chỉ khi có nơi lưu key (và chỉ ở lệnh
/// ghi cần đăng nhập — route mẫu ở đây không cần đăng nhập).
#[tokio::test]
async fn tai_lieu_khai_header_chung_tren_tung_endpoint() {
    let document = serde_json::to_value(api_with(&default_config()).openapi).unwrap();
    let operation = &document["paths"]["/api/v1/notes"]["post"];
    let headers: Vec<&str> = operation["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|parameter| parameter["in"] == "header")
        .map(|parameter| parameter["name"].as_str().unwrap())
        .collect();
    assert_eq!(headers, ["Accept-Language"]);
}
