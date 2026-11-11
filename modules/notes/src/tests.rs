//! Test của module mẫu, trên database gate đang chọn (gate chạy cả SQLite và Postgres).
//! Dựng module như binary dựng: migration → start → route + tool → lớp HTTP của kernel;
//! gọi qua REST và qua client MCP thật.

use std::sync::Arc;

use autoxide_kernel::access::{Authenticator, Principal};
use autoxide_kernel::config::{Config, ConfigReader, ConfigSource};
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::module::Modules;
use autoxide_testkit::TestDatabase;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use rmcp::ServiceExt as _;
use rmcp::model::CallToolRequestParams;
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use sea_orm::{ConnectionTrait, Statement};
use serde_json::{Value, json};
use tower::ServiceExt as _;
use uuid::Uuid;

use crate::api::Notes;
use crate::service::{NewNote, NoteChanges};

/// Hai người dùng: `ada` đọc và ghi được, `bob` chỉ đọc được.
const ADA: Uuid = Uuid::from_u128(0xada);
const BOB: Uuid = Uuid::from_u128(0xb0b);

struct FakeAuthenticator;

#[async_trait::async_trait]
impl Authenticator for FakeAuthenticator {
    async fn authenticate(&self, authorization: Option<&str>) -> Result<Principal, Error> {
        let both = ["notes.read".to_owned(), "notes.write".to_owned()];
        match authorization {
            Some("Bearer ada") => Ok(Principal::new(ADA, both)),
            Some("Bearer bob") => Ok(Principal::new(BOB, ["notes.read".to_owned()])),
            _ => Err(Error::internal("unknown test token")),
        }
    }
}

struct App {
    router: Router,
    openapi: utoipa::openapi::OpenApi,
    modules: Modules,
    database: Database,
}

async fn app(test: &TestDatabase) -> App {
    let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
    let mut reader = ConfigReader::new(&source);
    let config = Config::read(&mut reader);
    let (config, _) = reader.finish(config).unwrap();
    let database = Database::connect(&config.database).await.unwrap();
    let mut modules = Modules::new(vec![Box::new(crate::module())]).unwrap();
    modules.migrator().unwrap().up(&database).await.unwrap();
    modules.start(&database).unwrap();
    let extras = autoxide_kernel::http::Extras {
        authenticator: Some(Arc::new(FakeAuthenticator)),
        tools: modules.tools().unwrap(),
        translations: modules.translations().clone(),
        ..Default::default()
    };
    let api = autoxide_kernel::http::build(
        &config.http,
        utoipa_axum::router::OpenApiRouter::new(),
        modules.routes(),
        extras,
    );
    App { router: api.router, openapi: api.openapi, modules, database }
}

async fn rest(
    app: &App,
    method: &str,
    uri: &str,
    who: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(format!("/api/v1/notes{uri}"))
        .header(header::AUTHORIZATION, format!("Bearer {who}"))
        .header(header::ACCEPT_LANGUAGE, "vi")
        .header(header::CONTENT_TYPE, "application/json");
    let body = body.map_or_else(Body::empty, |body| Body::from(body.to_string()));
    let response = app.router.clone().oneshot(request.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap() };
    (status, json)
}

async fn mcp(app: &App, who: &str) -> RunningService<RoleClient, ()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let router = app.router.clone();
    tokio::spawn(async move { axum::serve(listener, router).await });
    autoxide_kernel::tls::install_crypto_provider();
    let config = StreamableHttpClientTransportConfig::with_uri(url).auth_header(who);
    ().serve(StreamableHttpClientTransport::from_config(config)).await.unwrap()
}

fn tool(name: &str, arguments: Value) -> CallToolRequestParams {
    let arguments = arguments.as_object().cloned().unwrap_or_default();
    CallToolRequestParams::new(name.to_owned()).with_arguments(arguments)
}

async fn events(database: &Database) -> Vec<String> {
    let backend = database.read().get_database_backend();
    let sql = "SELECT name FROM notes_events ORDER BY id";
    let rows = database.read().query_all_raw(Statement::from_string(backend, sql)).await;
    rows.unwrap().iter().map(|row| row.try_get("", "name").unwrap()).collect()
}

/// REST và MCP gọi cùng service: ghi bằng đường này, đọc được bằng đường kia.
#[tokio::test]
async fn rest_va_mcp_dung_chung_service() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let ada = mcp(&app, "ada").await;

        let body = json!({"title": "Shopping", "body": "Milk"});
        let (status, created) = rest(&app, "POST", "", "ada", Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let listed = ada.call_tool(tool("notes.list_notes", json!({}))).await.unwrap();
        let listed = listed.structured_content.unwrap();
        assert_eq!(listed["data"]["data"][0]["id"], created["data"]["id"]);

        let made = ada.call_tool(tool("notes.create_note", json!({"title": "Ideas"}))).await;
        let made = made.unwrap().structured_content.unwrap();
        let id = made["data"]["id"].as_str().unwrap();
        let (status, got) = rest(&app, "GET", &format!("/{id}"), "ada", None).await;
        assert_eq!((status, got["data"]["title"].as_str()), (StatusCode::OK, Some("Ideas")));

        // Cùng lỗi, cùng mã, qua cả hai đường.
        let taken = ada.call_tool(tool("notes.create_note", json!({"title": "Ideas"}))).await;
        let taken = taken.unwrap();
        assert_eq!(taken.is_error, Some(true));
        assert_eq!(taken.structured_content.unwrap()["code"], "notes.title_taken");
        let (status, problem) =
            rest(&app, "POST", "", "ada", Some(json!({"title": "Ideas"}))).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::CONFLICT, Some("notes.title_taken"))
        );
        assert_eq!(problem["detail"], "Bạn đã có một ghi chú tên `Ideas`. Hãy chọn tên khác.");

        let updated =
            ada.call_tool(tool("notes.update_note", json!({"id": id, "body": "More"}))).await;
        assert_eq!(updated.unwrap().structured_content.unwrap()["data"]["body"], "More");
        let deleted = ada.call_tool(tool("notes.delete_note", json!({"id": id}))).await.unwrap();
        assert_eq!(deleted.is_error, Some(false));
        let (status, _) = rest(&app, "GET", &format!("/{id}"), "ada", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // Mỗi lệnh ghi thành công đúng một event (audit log đọc từ đây).
        assert_eq!(
            events(&app.database).await,
            [
                "notes.note_created",
                "notes.note_created",
                "notes.note_updated",
                "notes.note_deleted"
            ]
        );
        ada.cancel().await.unwrap();
    })
    .await
    .unwrap();
}

/// Người chỉ có `notes.read`: không thấy tool ghi, REST ghi bị 403; không thấy ghi chú
/// của người khác.
#[tokio::test]
async fn permission_va_du_lieu_rieng_tung_nguoi() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let (_, created) = rest(&app, "POST", "", "ada", Some(json!({"title": "Mine"}))).await;
        let id = created["data"]["id"].as_str().unwrap();

        let bob = mcp(&app, "bob").await;
        let tools = bob.list_all_tools().await.unwrap();
        let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_ref()).collect();
        assert_eq!(names, ["notes.list_notes", "notes.get_note"]);
        let (status, problem) = rest(&app, "POST", "", "bob", Some(json!({"title": "x"}))).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("permission_denied"))
        );

        let (status, _) = rest(&app, "GET", &format!("/{id}"), "bob", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let theirs = bob.call_tool(tool("notes.get_note", json!({"id": id}))).await.unwrap();
        assert_eq!(theirs.structured_content.unwrap()["code"], "notes.note_not_found");
        let (_, listed) = rest(&app, "GET", "", "bob", None).await;
        assert_eq!(listed["data"], json!([]));
        bob.cancel().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn kiem_dau_vao_nhu_nhau_o_rest_va_mcp() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let ada = mcp(&app, "ada").await;
        let long = "x".repeat(201);

        let (status, problem) = rest(&app, "POST", "", "ada", Some(json!({"title": long}))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(problem["errors"][0]["field"], "title");
        let invalid = ada.call_tool(tool("notes.create_note", json!({"title": long}))).await;
        let invalid = invalid.unwrap().structured_content.unwrap();
        assert_eq!(
            (invalid["code"].as_str(), invalid["errors"][0]["field"].as_str()),
            (Some("validation_failed"), Some("title"))
        );
        let blank = ada.call_tool(tool("notes.create_note", json!({"title": "   "}))).await;
        assert_eq!(blank.unwrap().structured_content.unwrap()["code"], "validation_failed");
        ada.cancel().await.unwrap();
    })
    .await
    .unwrap();
}

/// Cùng một người tạo cùng một tên từ nhiều request cùng lúc: đúng một cái thành công
/// (index unique quyết) — trên cả hai database.
#[tokio::test]
async fn tao_trung_ten_dong_thoi_chi_mot_thanh_cong() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let service = crate::service::NoteService::new(
            app.database.clone(),
            autoxide_kernel::event::Outbox::standalone("notes"),
        );
        let service = Arc::new(service);

        let attempts: Vec<_> = (0..6)
            .map(|_| {
                let service = service.clone();
                tokio::spawn(async move {
                    let input = NewNote { title: "Same".to_owned(), body: String::new() };
                    service.create(Actor::user(ADA), input).await
                })
            })
            .collect();
        let mut created = 0;
        for attempt in attempts {
            match attempt.await.unwrap() {
                Ok(_) => created += 1,
                Err(error) => assert_eq!(error.code(), "notes.title_taken", "{error:?}"),
            }
        }
        assert_eq!(created, 1);

        // Người khác dùng cùng tên được; đổi tên trùng thì bị chặn.
        let other = NewNote { title: "Same".to_owned(), body: String::new() };
        service.create(Actor::user(BOB), other).await.unwrap();
        let second = NewNote { title: "Second".to_owned(), body: String::new() };
        let second = service.create(Actor::user(ADA), second).await.unwrap();
        let rename = NoteChanges { title: Some("Same".to_owned()), body: None };
        let taken = service.update(Actor::user(ADA), second.id, rename).await.unwrap_err();
        assert_eq!(taken.code(), "notes.title_taken");
    })
    .await
    .unwrap();
}

/// Module khác tra ghi chú qua trait công bố, không đọc bảng của `notes`.
#[tokio::test]
async fn module_khac_dung_trait_cong_bo() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let (_, created) = rest(&app, "POST", "", "ada", Some(json!({"title": "Shared"}))).await;
        let id: Uuid = created["data"]["id"].as_str().unwrap().parse().unwrap();

        let notes = app.modules.api::<dyn Notes>().unwrap();

        let found = notes.find(id).await.unwrap().unwrap();
        assert_eq!((found.owner_id, found.title.as_str()), (ADA, "Shared"));
        assert!(notes.find(Uuid::now_v7()).await.unwrap().is_none());
    })
    .await
    .unwrap();
}

/// Tài liệu OpenAPI của module đạt luật của kernel (mỗi endpoint đủ mô tả để gọi đúng
/// mà không đọc mã nguồn) — như gate kiểm cho các module đã đăng ký.
#[tokio::test]
async fn tai_lieu_openapi_du_mo_ta() {
    TestDatabase::run(async |test| {
        let app = app(test).await;

        assert_eq!(autoxide_kernel::http::lint(&app.openapi), Vec::<String>::new());
        assert!(app.openapi.paths.paths.contains_key("/api/v1/notes/{id}"));
    })
    .await
    .unwrap();
}
