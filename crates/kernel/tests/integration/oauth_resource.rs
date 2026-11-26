//! Resource server OAuth của kernel (bước 71 G4, D-20261126-mcp-oauth): tài liệu khám phá RFC 9728 / RFC 8414,
//! `resource_metadata` trong `WWW-Authenticate`, kiểm audience (RFC 8707) và 403 `insufficient_scope` cho cả MCP lẫn REST.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use autoxide_kernel::access::{
    Authenticator, AuthorizationServer, OAuthAccess, Permission, PermissionCatalog, Principal,
};
use autoxide_kernel::config::{ConfigReader, ConfigSource, DatabaseConfig, HttpConfig};
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::http;
use autoxide_kernel::mcp::{NoInput, Tool};
use autoxide_kernel::module::{Module, Modules};
use autoxide_kernel::response::Data;
use autoxide_kernel::tenancy::{DatabaseRegistry, TenantContext};
use autoxide_testkit::TestDatabase;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode, header};
use serde_json::{Map, Value, json};
use tower::ServiceExt as _;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

const PUBLIC: &str = "https://api.example.test";
const NOTES_READ: Permission = Permission::new("notes.read", "Read notes.");
const NOTES_WRITE: Permission = Permission::new("notes.write", "Write notes.");

struct Notes;

impl Module for Notes {
    fn name(&self) -> &'static str {
        "notes"
    }

    fn permissions(&self) -> Vec<Permission> {
        vec![NOTES_READ, NOTES_WRITE]
    }

    fn tools(&self) -> Vec<Tool> {
        vec![
            Tool::new("list", "List notes.", |_: TenantContext, _: Principal, _: NoInput| async {
                Ok(json!([]))
            })
            .requires(NOTES_READ)
            .read_only(),
            Tool::new(
                "write",
                "Write a note.",
                |_: TenantContext, _: Principal, _: NoInput| async { Ok(json!({"written": true})) },
            )
            .requires(NOTES_WRITE),
        ]
    }
}

/// Write a note.
#[utoipa::path(
    post,
    path = "/",
    responses((status = 200, description = "Written.", body = Object)),
)]
async fn write_note(principal: Principal) -> Result<Data<Value>, Error> {
    principal.require(&NOTES_WRITE)?;
    Ok(Data { data: json!({"written": true}) })
}

/// Máy chủ uỷ quyền giả: metadata cố định, `issuer` do kernel đặt.
struct FakeServer;

impl AuthorizationServer for FakeServer {
    fn metadata(&self, issuer: &str) -> Map<String, Value> {
        let mut document = Map::new();
        document.insert("issuer".to_owned(), json!("https://wrong.example"));
        document.insert("token_endpoint".to_owned(), json!(format!("{issuer}/token")));
        document.insert("code_challenge_methods_supported".to_owned(), json!(["S256"]));
        document
    }
}

/// Người dùng có `notes.read` và `notes.write`. Token:
/// - `mcp`: grant cho `/mcp`, scope `notes.*`;
/// - `mcp-read`: grant cho `/mcp`, scope chỉ `notes.read`;
/// - `rest-read`: grant cho `/api/v1`, scope chỉ `notes.read`;
/// - `notes-only`: grant cho `/mcp/notes`;
/// - `session`: phiên đăng nhập thường (không audience).
struct FakeAuthenticator;

fn user() -> Principal {
    Principal::new(Uuid::nil(), ["notes.read".to_owned(), "notes.write".to_owned()])
}

fn granted(scopes: &[&str], audience: &str) -> Principal {
    let catalog = PermissionCatalog::new(vec![("notes", vec![NOTES_READ, NOTES_WRITE])]).unwrap();
    let scopes: Vec<String> = scopes.iter().map(|scope| (*scope).to_owned()).collect();
    user().within_scope(&scopes, &catalog).with_oauth(OAuthAccess::new(
        Uuid::now_v7(),
        "https://client.example/app.json",
        audience,
    ))
}

#[async_trait::async_trait]
impl Authenticator for FakeAuthenticator {
    async fn authenticate(
        &self,
        authorization: Option<&str>,
        _: &autoxide_kernel::tenancy::TenantHint,
    ) -> Result<Principal, Error> {
        match authorization {
            Some("Bearer mcp") => Ok(granted(&["notes.*"], &format!("{PUBLIC}/mcp"))),
            Some("Bearer mcp-read") => Ok(granted(&["notes.read"], &format!("{PUBLIC}/mcp"))),
            Some("Bearer rest-read") => Ok(granted(&["notes.read"], &format!("{PUBLIC}/api/v1"))),
            Some("Bearer notes-only") => Ok(granted(&["*"], &format!("{PUBLIC}/mcp/notes"))),
            Some("Bearer session") => Ok(user()),
            _ => Err(Error::unauthenticated("invalid_token", "The token is not valid.")),
        }
    }
}

async fn router(test: &TestDatabase, oauth: bool) -> axum::Router {
    let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
    let mut reader = ConfigReader::new(&source);
    let config = HttpConfig::read(&mut reader);
    let database = DatabaseConfig::read(&mut reader);
    let (config, database) = reader
        .finish(config.and_then(|config| database.map(|database| (config, database))))
        .unwrap()
        .0;
    let database = Database::connect(&database).await.unwrap();
    let modules = Modules::new(vec![Box::new(Notes)]).unwrap();
    let mut extras = http::Extras::default()
        .authenticator(Some(Arc::new(FakeAuthenticator)))
        .tools(modules.tools().unwrap())
        .tenancy(DatabaseRegistry::single(database));
    if oauth {
        extras = extras
            .public_url(Some(PUBLIC.to_owned()))
            .authorization_server(Some(Arc::new(FakeServer)));
    }
    let notes = OpenApiRouter::new().routes(routes!(write_note));
    http::build(&config, axum::Router::new().into(), vec![("notes", notes)], extras).router
}

struct Answer {
    status: StatusCode,
    headers: HeaderMap,
    body: Value,
}

impl Answer {
    fn challenge(&self) -> &str {
        self.headers.get(header::WWW_AUTHENTICATE).map_or("", |value| value.to_str().unwrap())
    }
}

async fn send(router: axum::Router, request: Request<Body>) -> Answer {
    let response = router.oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    Answer { status, headers, body: serde_json::from_slice(&bytes).unwrap_or(Value::Null) }
}

#[test]
fn token_oauth_khong_phai_phien_dang_nhap() {
    use autoxide_kernel::access::Credential;
    let oauth = granted(&["notes.*"], &format!("{PUBLIC}/mcp"));
    assert!(!oauth.is_session());
    assert!(matches!(oauth.credential(), Credential::OAuthGrant(_)));
    assert!(oauth.credential_id().is_some());
    assert!(user().is_session());
    let key = Uuid::now_v7();
    assert_eq!(user().via_api_key(key).credential(), Credential::ApiKey(key));
}

async fn get(router: axum::Router, path: &str) -> Answer {
    send(
        router,
        Request::get(path).header(header::HOST, "api.example.test").body(Body::empty()).unwrap(),
    )
    .await
}

async fn mcp(router: axum::Router, path: &str, token: Option<&str>, message: Value) -> Answer {
    let mut request = Request::post(path)
        .header(header::HOST, "api.example.test")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream");
    if let Some(token) = token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    send(router, request.body(Body::from(message.to_string())).unwrap()).await
}

fn list_tools() -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})
}

fn call(name: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": name, "arguments": {}}})
}

#[tokio::test]
async fn tai_lieu_kham_pha_theo_rfc_9728_va_8414() {
    TestDatabase::run(async |test| {
        let app = router(test, true).await;

        // Không có tài liệu gốc: token cho cả máy chủ không bao giờ được cấp (chỉ `/mcp`, `/mcp/<module>`, `/api/v1`).
        let root = get(app.clone(), "/.well-known/oauth-protected-resource").await;
        assert_eq!(root.status, StatusCode::NOT_FOUND);

        let mcp = get(app.clone(), "/.well-known/oauth-protected-resource/mcp").await;
        assert_eq!(mcp.status, StatusCode::OK);
        assert_eq!(mcp.body["resource"], format!("{PUBLIC}/mcp"));
        assert_eq!(mcp.body["authorization_servers"], json!([PUBLIC]));
        assert_eq!(mcp.body["bearer_methods_supported"], json!(["header"]));
        assert!(mcp.body.get("scopes_supported").is_none());
        assert_eq!(mcp.headers[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
        let notes = get(app.clone(), "/.well-known/oauth-protected-resource/mcp/notes").await;
        assert_eq!(notes.body["resource"], format!("{PUBLIC}/mcp/notes"));
        let rest = get(app.clone(), "/.well-known/oauth-protected-resource/api/v1").await;
        assert_eq!(rest.body["resource"], format!("{PUBLIC}/api/v1"));
        let unknown = get(app.clone(), "/.well-known/oauth-protected-resource/mcp/nothing").await;
        assert_eq!(unknown.status, StatusCode::NOT_FOUND);

        // `issuer` luôn là địa chỉ đã tải, dù module khai sai.
        let server = get(app.clone(), "/.well-known/oauth-authorization-server").await;
        assert_eq!(server.status, StatusCode::OK);
        assert_eq!(server.body["issuer"], PUBLIC);
        assert_eq!(server.body["token_endpoint"], format!("{PUBLIC}/token"));

        // `Host` lạ không thành issuer (không phản chiếu host của kẻ tấn công).
        let foreign = send(
            app,
            Request::get("/.well-known/oauth-authorization-server")
                .header(header::HOST, "attacker.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(foreign.body["issuer"], PUBLIC);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn chua_bat_oauth_thi_khong_co_tai_lieu_va_header_giu_nguyen() {
    TestDatabase::run(async |test| {
        let app = router(test, false).await;
        let missing = get(app.clone(), "/.well-known/oauth-protected-resource/mcp").await;
        assert_eq!(missing.status, StatusCode::NOT_FOUND);
        assert_eq!(missing.body["code"], "oauth_unavailable");
        let unauthenticated = mcp(app, "/mcp", None, list_tools()).await;
        assert_eq!(unauthenticated.status, StatusCode::UNAUTHORIZED);
        assert_eq!(unauthenticated.challenge(), "Bearer");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khong_token_thi_401_tro_toi_metadata_dung_duong_dan() {
    TestDatabase::run(async |test| {
        let app = router(test, true).await;
        let root = mcp(app.clone(), "/mcp", None, list_tools()).await;
        assert_eq!(root.status, StatusCode::UNAUTHORIZED);
        assert_eq!(
            root.challenge(),
            format!(
                "Bearer resource_metadata=\"{PUBLIC}/.well-known/oauth-protected-resource/mcp\""
            )
        );
        let notes = mcp(app.clone(), "/mcp/notes", None, list_tools()).await;
        assert!(notes.challenge().ends_with("/.well-known/oauth-protected-resource/mcp/notes\""));
        let rest = send(
            app,
            Request::post("/api/v1/notes")
                .header(header::HOST, "api.example.test")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(rest.status, StatusCode::UNAUTHORIZED);
        assert!(rest.challenge().ends_with("/.well-known/oauth-protected-resource/api/v1\""));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn token_chi_dung_duoc_o_resource_duoc_cap() {
    TestDatabase::run(async |test| {
        let app = router(test, true).await;
        // `/mcp` phủ `/mcp/notes`.
        assert_eq!(
            mcp(app.clone(), "/mcp", Some("mcp"), list_tools()).await.status,
            StatusCode::OK
        );
        assert_eq!(
            mcp(app.clone(), "/mcp/notes", Some("mcp"), list_tools()).await.status,
            StatusCode::OK
        );
        // Hẹp không phủ rộng.
        let narrow = mcp(app.clone(), "/mcp", Some("notes-only"), list_tools()).await;
        assert_eq!(narrow.status, StatusCode::UNAUTHORIZED);
        assert!(narrow.challenge().starts_with("Bearer error=\"invalid_token\""));
        assert!(narrow.challenge().contains("resource_metadata="));
        // Token của REST không dùng cho MCP, và ngược lại.
        let rest_at_mcp = mcp(app.clone(), "/mcp", Some("rest-read"), list_tools()).await;
        assert_eq!(rest_at_mcp.status, StatusCode::UNAUTHORIZED);
        let mcp_at_rest = send(
            app.clone(),
            Request::post("/api/v1/notes")
                .header(header::HOST, "api.example.test")
                .header(header::AUTHORIZATION, "Bearer mcp")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(mcp_at_rest.status, StatusCode::UNAUTHORIZED);
        assert_eq!(mcp_at_rest.body["code"], "oauth_token_audience");
        // Biến thể đường dẫn: `/` cuối, ký tự mã hoá — hoặc không tới được route, hoặc bị từ chối; không bao giờ
        // 200 với token sai audience.
        let slash = mcp(app.clone(), "/mcp/notes/", Some("rest-read"), list_tools()).await;
        assert_ne!(slash.status, StatusCode::OK);
        let encoded = mcp(app.clone(), "/mcp/%6eotes", Some("rest-read"), list_tools()).await;
        assert_ne!(encoded.status, StatusCode::OK);
        let rest_slash = send(
            app.clone(),
            Request::post("/api/v1/notes/")
                .header(header::HOST, "api.example.test")
                .header(header::AUTHORIZATION, "Bearer notes-only")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_ne!(rest_slash.status, StatusCode::OK);
        // `/mcp/notes/` với token đúng resource vẫn dùng được nếu router nhận nó.
        let ok = mcp(app.clone(), "/mcp/notes", Some("notes-only"), list_tools()).await;
        assert_eq!(ok.status, StatusCode::OK);
        // Phiên đăng nhập (không audience) vẫn như cũ.
        assert_eq!(mcp(app, "/mcp", Some("session"), list_tools()).await.status, StatusCode::OK);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn thieu_scope_ma_nguoi_dung_co_quyen_thi_403_insufficient_scope() {
    TestDatabase::run(async |test| {
        let app = router(test, true).await;
        let allowed = mcp(app.clone(), "/mcp", Some("mcp-read"), call("notes.list")).await;
        assert_eq!(allowed.status, StatusCode::OK);
        assert_eq!(allowed.body["result"]["isError"], false);

        let refused = mcp(app.clone(), "/mcp", Some("mcp-read"), call("notes.write")).await;
        assert_eq!(refused.status, StatusCode::FORBIDDEN);
        let challenge = refused.challenge();
        assert!(challenge.starts_with("Bearer error=\"insufficient_scope\""), "{challenge}");
        assert!(challenge.contains("scope=\"notes.write\""), "{challenge}");
        assert!(
            challenge.contains(&format!(
                "resource_metadata=\"{PUBLIC}/.well-known/oauth-protected-resource/mcp\""
            )),
            "{challenge}"
        );
        // Thân vẫn là kết quả lỗi của tool cho client không làm step-up, kèm challenge trong `_meta` (ChatGPT).
        assert_eq!(refused.body["result"]["isError"], true);
        assert_eq!(refused.body["result"]["_meta"]["mcp/www_authenticate"], json!([challenge]));

        let rest = send(
            app,
            Request::post("/api/v1/notes")
                .header(header::HOST, "api.example.test")
                .header(header::AUTHORIZATION, "Bearer rest-read")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(rest.status, StatusCode::FORBIDDEN, "{}", rest.body);
        assert_eq!(rest.body["reason"], "outside_grant_scope");
        assert!(rest.challenge().contains("insufficient_scope"));
        assert!(rest.challenge().contains("/.well-known/oauth-protected-resource/api/v1\""));
    })
    .await
    .unwrap();
}
