//! Endpoint MCP qua HTTP, gọi bằng client MCP thật (`rmcp`): liệt kê và gọi tool theo
//! permission, lỗi của tool, xác thực.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use autoxide_kernel::access::{Authenticator, Permission, Principal};
use autoxide_kernel::config::{ConfigReader, ConfigSource, DatabaseConfig, HttpConfig};
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::{self, MCP_PATH};
use autoxide_kernel::mcp::{NoInput, Tool};
use autoxide_kernel::module::{Module, Modules};
use autoxide_kernel::tenancy::{DatabaseRegistry, TenantContext};
use autoxide_testkit::TestDatabase;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use rmcp::ServiceExt as _;
use rmcp::model::CallToolRequestParams;
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use serde::Deserialize;
use serde_json::{Value, json};
use tower::ServiceExt as _;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

const NOTES_WRITE: Permission = Permission::new("notes.write", "Write notes.");

/// Two numbers to add.
#[derive(Deserialize, Validate, ToSchema)]
struct AddInput {
    /// The first number, 0 to 100.
    #[validate(range(min = 0, max = 100))]
    a: i64,
    b: i64,
}

struct Notes;

impl Module for Notes {
    fn name(&self) -> &'static str {
        "notes"
    }

    fn permissions(&self) -> Vec<Permission> {
        vec![NOTES_WRITE]
    }

    fn tools(&self) -> Vec<Tool> {
        vec![
            Tool::new(
                "whoami",
                "Tell who is calling.",
                |_: TenantContext, principal: Principal, _: NoInput| async move {
                    Ok(json!({ "user_id": principal.user_id() }))
                },
            )
            .read_only(),
            Tool::new(
                "add",
                "Add two numbers.",
                |_: TenantContext, _: Principal, input: AddInput| async move {
                    if input.b == 13 {
                        return Err(Error::conflict("notes.unlucky", "13 is not allowed."));
                    }
                    Ok(input.a + input.b)
                },
            )
            .requires(NOTES_WRITE),
            Tool::new(
                "issue_key",
                "Issue a secret.",
                |_: TenantContext, _: Principal, _: NoInput| async move {
                    Ok(json!({ "key": "secret-once" }))
                },
            )
            .returns_secret(),
            // Tool cũ, deprecated, thay bằng `whoami` (D-240).
            Tool::new(
                "who",
                "Tell who is calling.",
                |_: TenantContext, principal: Principal, _: NoInput| async move {
                    Ok(json!({ "user_id": principal.user_id() }))
                },
            )
            .read_only(),
        ]
    }

    fn deprecations(&self) -> Vec<http::Deprecation> {
        vec![http::Deprecation::tool("who", "2026-11-01").sunset("2027-05-01").successor("whoami")]
    }
}

/// `Bearer writer` có `notes.write`; `Bearer reader` không có permission nào; còn lại
/// là token sai.
struct FakeAuthenticator;

#[async_trait::async_trait]
impl Authenticator for FakeAuthenticator {
    async fn authenticate(
        &self,
        authorization: Option<&str>,
        _: &autoxide_kernel::tenancy::TenantHint,
    ) -> Result<Principal, Error> {
        match authorization {
            Some("Bearer writer") => Ok(Principal::new(Uuid::nil(), ["notes.write".to_owned()])),
            Some("Bearer reader") => Ok(Principal::new(Uuid::max(), [])),
            _ => Err(Error::unauthenticated("invalid_token", "The token is not valid.")),
        }
    }
}

/// App chỉ có endpoint MCP; khách của request là khách mặc định trên database của test.
async fn router(test: &TestDatabase) -> axum::Router {
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
    let extras = http::Extras::default()
        .authenticator(Some(Arc::new(FakeAuthenticator)))
        .tools(modules.tools().unwrap())
        .tenancy(DatabaseRegistry::single(database));
    http::build(&config, axum::Router::new().into(), Vec::new(), extras).router
}

/// Máy chủ thật trên một port ngẫu nhiên; trả URL của endpoint MCP.
async fn serve(test: &TestDatabase) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = router(test).await;
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{address}{MCP_PATH}")
}

async fn client(url: &str, token: &str) -> RunningService<RoleClient, ()> {
    autoxide_kernel::tls::install_crypto_provider();
    let config = StreamableHttpClientTransportConfig::with_uri(url.to_owned()).auth_header(token);
    ().serve(StreamableHttpClientTransport::from_config(config)).await.unwrap()
}

fn arguments(value: Value) -> serde_json::Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => panic!("arguments must be an object"),
    }
}

#[tokio::test]
async fn chi_liet_ke_va_goi_duoc_tool_co_permission() {
    TestDatabase::run(async |test| {
        let url = serve(test).await;
        let writer = client(&url, "writer").await;
        let reader = client(&url, "reader").await;

        let names = |tools: Vec<rmcp::model::Tool>| -> Vec<String> {
            tools.into_iter().map(|tool| tool.name.into_owned()).collect()
        };
        let tools = writer.list_all_tools().await.unwrap();
        let add = tools.iter().find(|tool| tool.name == "notes.add").unwrap();
        assert!(
            add.description.as_deref().unwrap().ends_with("Requires permission `notes.write`.")
        );
        assert_eq!(add.input_schema["properties"]["a"]["type"], "integer");
        assert_eq!(
            names(tools),
            [
                "kernel.list_custom_fields",
                "notes.whoami",
                "notes.add",
                "notes.issue_key",
                "notes.who"
            ]
        );
        assert_eq!(
            names(reader.list_all_tools().await.unwrap()),
            ["kernel.list_custom_fields", "notes.whoami", "notes.issue_key", "notes.who"]
        );

        let call = CallToolRequestParams::new("notes.add")
            .with_arguments(arguments(json!({"a": 2, "b": 3})));
        let result = writer.call_tool(call.clone()).await.unwrap();
        assert_eq!(result.is_error, Some(false));
        assert_eq!(result.structured_content, Some(json!({"data": 5})));
        // Không có permission: như tool không tồn tại.
        let denied = reader.call_tool(call).await.unwrap_err().to_string();
        assert!(denied.contains("Unknown tool `notes.add`"), "{denied}");

        let whoami = reader.call_tool(CallToolRequestParams::new("notes.whoami")).await.unwrap();
        assert_eq!(whoami.structured_content, Some(json!({"data": {"user_id": Uuid::max()}})));
        writer.cancel().await.unwrap();
        reader.cancel().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn loi_cua_tool_co_code_va_thong_bao() {
    TestDatabase::run(async |test| {
        let url = serve(test).await;
        let writer = client(&url, "writer").await;

        let call =
            |value: Value| CallToolRequestParams::new("notes.add").with_arguments(arguments(value));
        let invalid = writer.call_tool(call(json!({"a": 500, "b": 1}))).await.unwrap();
        assert_eq!(invalid.is_error, Some(true));
        let body = invalid.structured_content.unwrap();
        assert_eq!(body["code"], "validation_failed");
        assert_eq!(body["errors"][0]["field"], "a");
        let missing = writer.call_tool(call(json!({"a": 1}))).await.unwrap();
        assert_eq!(missing.structured_content.unwrap()["code"], "validation_failed");
        let conflict = writer.call_tool(call(json!({"a": 1, "b": 13}))).await.unwrap();
        let body = conflict.structured_content.unwrap();
        assert_eq!(
            (body["code"].as_str(), body["message"].as_str()),
            (Some("notes.unlucky"), Some("13 is not allowed."))
        );
        writer.cancel().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn thieu_hay_sai_token_thi_401() {
    TestDatabase::run(async |test| {
        for authorization in [None, Some("Bearer wrong")] {
            let mut request = Request::builder()
                .method("POST")
                .uri(MCP_PATH)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ACCEPT, "application/json, text/event-stream");
            if let Some(value) = authorization {
                request = request.header(header::AUTHORIZATION, value);
            }
            let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}).to_string();
            let response =
                router(test).await.oneshot(request.body(Body::from(body)).unwrap()).await.unwrap();

            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            assert_eq!(response.headers()[header::WWW_AUTHENTICATE], "Bearer");
        }
    })
    .await
    .unwrap();
}

/// Tool trả secret: response mang `Cache-Control: no-store` (không ai lưu lại, kể cả lớp
/// idempotency); tool khác thì không (D-149).
#[tokio::test]
async fn tool_tra_secret_thi_response_no_store() {
    TestDatabase::run(async |test| {
        for (tool, expected) in [("notes.issue_key", Some("no-store")), ("notes.whoami", None)] {
            let body = json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": {"name": tool, "arguments": {}}
            });
            let request = Request::builder()
                .method("POST")
                .uri(MCP_PATH)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ACCEPT, "application/json, text/event-stream")
                .header(header::AUTHORIZATION, "Bearer writer")
                .header(header::HOST, "localhost")
                .body(Body::from(body.to_string()))
                .unwrap();
            let response = router(test).await.oneshot(request).await.unwrap();

            let status = response.status();
            let headers = response.headers().clone();
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
            assert_eq!(status, StatusCode::OK, "{tool}: {}", String::from_utf8_lossy(&bytes));
            let cache = headers.get(header::CACHE_CONTROL);
            assert_eq!(cache.map(|value| value.to_str().unwrap()), expected, "{tool}");
        }
    })
    .await
    .unwrap();
}

/// Tool deprecated: `tools/list` nói rõ, kết quả giữ envelope và kèm cảnh báo cho máy (`_meta`) lẫn
/// cho mô hình (đoạn văn bản) (D-240).
#[tokio::test]
async fn tool_deprecated_co_cau_deprecated_va_canh_bao_trong_ket_qua() {
    TestDatabase::run(async |test| {
        let url = serve(test).await;
        let reader = client(&url, "reader").await;
        let tools = reader.list_all_tools().await.unwrap();
        let who = tools.iter().find(|tool| tool.name == "notes.who").unwrap();
        let description = who.description.as_deref().unwrap();
        assert!(
            description.ends_with(
                "Tool `notes.who` is deprecated since 2026-11-01. It will be removed on 2027-05-01. \
                 Use `notes.whoami` instead."
            ),
            "{description}"
        );
        let whoami = tools.iter().find(|tool| tool.name == "notes.whoami").unwrap();
        assert!(!whoami.description.as_deref().unwrap().contains("deprecated"));

        let result = reader.call_tool(CallToolRequestParams::new("notes.who")).await.unwrap();
        assert_eq!(result.structured_content, Some(json!({"data": {"user_id": Uuid::max()}})));
        let meta = result.meta.unwrap();
        assert_eq!(
            meta.0["autoxide/deprecation"],
            json!({"since": "2026-11-01", "sunset": "2027-05-01", "successor": "notes.whoami"})
        );
        let warning = serde_json::to_string(&result.content).unwrap();
        assert!(warning.contains("Warning: Tool `notes.who` is deprecated"), "{warning}");
        let plain = reader.call_tool(CallToolRequestParams::new("notes.whoami")).await.unwrap();
        assert!(plain.meta.is_none());
        reader.cancel().await.unwrap();
    })
    .await
    .unwrap();
}

#[test]
fn khai_deprecation_cho_tool_khong_co_bi_tu_choi_luc_khoi_dong() {
    struct Wrong;
    impl Module for Wrong {
        fn name(&self) -> &'static str {
            "wrong"
        }
        fn tools(&self) -> Vec<Tool> {
            vec![Tool::new("real", "x", |_: TenantContext, _: Principal, _: NoInput| async move {
                Ok(())
            })]
        }
        fn deprecations(&self) -> Vec<http::Deprecation> {
            vec![
                http::Deprecation::tool("ghost", "2026-11-01"),
                http::Deprecation::tool("real", "2026-11-01").successor("missing"),
                http::Deprecation::tool("real", "not-a-date"),
            ]
        }
    }

    // Ngày sai: từ chối ngay khi dựng sổ module, như endpoint.
    let error = Modules::new(vec![Box::new(Wrong)]).unwrap_err().to_string();
    assert!(error.contains("deprecation of tool `real` of module `wrong`: `since`"), "{error}");

    struct Missing;
    impl Module for Missing {
        fn name(&self) -> &'static str {
            "wrong"
        }
        fn tools(&self) -> Vec<Tool> {
            vec![Tool::new("real", "x", |_: TenantContext, _: Principal, _: NoInput| async move {
                Ok(())
            })]
        }
        fn deprecations(&self) -> Vec<http::Deprecation> {
            vec![
                http::Deprecation::tool("ghost", "2026-11-01"),
                http::Deprecation::tool("real", "2026-11-01").successor("missing"),
            ]
        }
    }
    let error = Modules::new(vec![Box::new(Missing)]).unwrap().tools().unwrap_err().to_string();
    assert!(error.contains("tool deprecation `ghost`"), "{error}");
    assert!(error.contains("tool successor `missing`"), "{error}");
}

#[test]
fn khai_tool_sai_bi_tu_choi() {
    struct Broken;
    impl Module for Broken {
        fn name(&self) -> &'static str {
            "broken"
        }
        fn tools(&self) -> Vec<Tool> {
            let noop = |_: TenantContext, _: Principal, _: NoInput| async move { Ok(()) };
            vec![
                Tool::new("Bad-Name", "x", noop),
                Tool::new("twice", "x", noop),
                Tool::new("twice", "x", noop),
                Tool::new("locked", "x", noop).requires(NOTES_WRITE),
                Tool::new("either", "x", noop)
                    .requires(NOTES_WRITE)
                    .or_requires(Permission::new("notes.write_own", "x")),
            ]
        }
    }

    let error = Modules::new(vec![Box::new(Broken)]).unwrap().tools().unwrap_err().to_string();

    assert!(error.contains("tool `Bad-Name` of module `broken`"), "{error}");
    assert!(error.contains("tool `broken.twice` is declared twice"), "{error}");
    assert!(
        error.contains("requires permission `notes.write`, which no module declares"),
        "{error}"
    );
    assert!(
        error.contains("requires permission `notes.write_own`, which no module declares"),
        "{error}"
    );
}

/// Tool có permission thay thế: có một trong hai là thấy và gọi được.
#[test]
fn permission_thay_the() {
    let own = Permission::new("notes.write_own", "Write own notes.");
    let noop = |_: TenantContext, _: Principal, _: NoInput| async move { Ok(()) };
    let tool = Tool::new("edit", "x", noop).requires(NOTES_WRITE).or_requires(own);
    let with = |permission: &str| Principal::new(Uuid::nil(), [permission.to_owned()]);
    assert!(tool.allows(&with("notes.write")));
    assert!(tool.allows(&with("notes.write_own")));
    assert!(!tool.allows(&with("notes.read")));
    assert_eq!(tool.alternative().map(|permission| permission.name()), Some("notes.write_own"));
}

/// Tool cần duyệt (bước 51 P12): agent gọi thì bị từ chối `approval_required`, người thì chạy.
#[tokio::test]
async fn tool_can_duyet_tu_choi_agent() {
    TestDatabase::run(async |test| {
        let tool = Tool::new("purge", "Purge.", |_, _, _: NoInput| async { Ok(true) })
            .requires_approval_for_agent();
        let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
        let mut reader = ConfigReader::new(&source);
        let database = DatabaseConfig::read(&mut reader);
        let database = reader.finish(database).unwrap().0;
        let database = Database::connect(&database).await.unwrap();
        let tenant = DatabaseRegistry::single(database).default_tenant();
        let person = Principal::new(Uuid::now_v7(), Vec::<String>::new());
        let agent = person.clone().with_actor_kind(autoxide_kernel::access::ActorKind::Agent);
        let refused = tool.call(tenant.clone(), agent, Value::Null).await.unwrap_err();
        assert_eq!(refused.code(), "approval_required");
        assert_eq!(tool.call(tenant, person, Value::Null).await.unwrap(), Value::Bool(true));
    })
    .await
    .unwrap();
}
