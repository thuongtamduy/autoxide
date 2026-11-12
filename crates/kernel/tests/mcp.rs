//! Endpoint MCP qua HTTP, gọi bằng client MCP thật (`rmcp`): liệt kê và gọi tool theo
//! permission, lỗi của tool, xác thực.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use autoxide_kernel::access::{Authenticator, Permission, Principal};
use autoxide_kernel::config::{ConfigReader, ConfigSource, HttpConfig};
use autoxide_kernel::error::Error;
use autoxide_kernel::http::{self, MCP_PATH};
use autoxide_kernel::mcp::{NoInput, Tool};
use autoxide_kernel::module::{Module, Modules};
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
                |principal: Principal, _: NoInput| async move {
                    Ok(json!({ "user_id": principal.user_id() }))
                },
            )
            .read_only(),
            Tool::new("add", "Add two numbers.", |_: Principal, input: AddInput| async move {
                if input.b == 13 {
                    return Err(Error::conflict("notes.unlucky", "13 is not allowed."));
                }
                Ok(input.a + input.b)
            })
            .requires(NOTES_WRITE),
        ]
    }
}

/// `Bearer writer` có `notes.write`; `Bearer reader` không có permission nào; còn lại
/// là token sai.
struct FakeAuthenticator;

#[async_trait::async_trait]
impl Authenticator for FakeAuthenticator {
    async fn authenticate(&self, authorization: Option<&str>) -> Result<Principal, Error> {
        match authorization {
            Some("Bearer writer") => Ok(Principal::new(Uuid::nil(), ["notes.write".to_owned()])),
            Some("Bearer reader") => Ok(Principal::new(Uuid::max(), [])),
            _ => Err(Error::unauthenticated("invalid_token", "The token is not valid.")),
        }
    }
}

fn router() -> axum::Router {
    let source = ConfigSource::default();
    let mut reader = ConfigReader::new(&source);
    let config = HttpConfig::read(&mut reader);
    let (config, _) = reader.finish(config).unwrap();
    let modules = Modules::new(vec![Box::new(Notes)]).unwrap();
    let extras = http::Extras {
        authenticator: Some(Arc::new(FakeAuthenticator)),
        tools: modules.tools().unwrap(),
        ..Default::default()
    };
    http::build(&config, axum::Router::new().into(), Vec::new(), extras).router
}

/// Máy chủ thật trên một port ngẫu nhiên; trả URL của endpoint MCP.
async fn serve() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router()).await.unwrap() });
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
    let url = serve().await;
    let writer = client(&url, "writer").await;
    let reader = client(&url, "reader").await;

    let names = |tools: Vec<rmcp::model::Tool>| -> Vec<String> {
        tools.into_iter().map(|tool| tool.name.into_owned()).collect()
    };
    let tools = writer.list_all_tools().await.unwrap();
    let add = tools.iter().find(|tool| tool.name == "notes.add").unwrap();
    assert!(add.description.as_deref().unwrap().ends_with("Requires permission `notes.write`."));
    assert_eq!(add.input_schema["properties"]["a"]["type"], "integer");
    assert_eq!(names(tools), ["notes.whoami", "notes.add"]);
    assert_eq!(names(reader.list_all_tools().await.unwrap()), ["notes.whoami"]);

    let call =
        CallToolRequestParams::new("notes.add").with_arguments(arguments(json!({"a": 2, "b": 3})));
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
}

#[tokio::test]
async fn loi_cua_tool_co_code_va_thong_bao() {
    let url = serve().await;
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
}

#[tokio::test]
async fn thieu_hay_sai_token_thi_401() {
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
        let response = router().oneshot(request.body(Body::from(body)).unwrap()).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(response.headers()[header::WWW_AUTHENTICATE], "Bearer");
    }
}

#[test]
fn khai_tool_sai_bi_tu_choi() {
    struct Broken;
    impl Module for Broken {
        fn name(&self) -> &'static str {
            "broken"
        }
        fn tools(&self) -> Vec<Tool> {
            let noop = |_: Principal, _: NoInput| async move { Ok(()) };
            vec![
                Tool::new("Bad-Name", "x", noop),
                Tool::new("twice", "x", noop),
                Tool::new("twice", "x", noop),
                Tool::new("locked", "x", noop).requires(NOTES_WRITE),
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
}
