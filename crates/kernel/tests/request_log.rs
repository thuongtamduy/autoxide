//! Nhật ký request theo khách (bước 52 mục 1, D-202): ghi đường dẫn mẫu, không ghi thân, query
//! hay token; lệnh MCP ghi `mcp:<tool>`, không ghi tham số; lọc theo `actor_kind` và API key.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use autoxide_kernel::access::{ActorKind, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::module::Module;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_testkit::{TestApp, TestDatabase, tool};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

/// Something private.
#[derive(Deserialize, Validate, ToSchema)]
struct EchoInput {
    /// A value that must never reach the log.
    secret: String,
}

struct Shop;

impl Module for Shop {
    fn name(&self) -> &'static str {
        "shop"
    }

    fn tools(&self) -> Vec<Tool> {
        vec![
            Tool::new(
                "echo",
                "Echo a value.",
                |_: TenantContext, _: Principal, input: EchoInput| async move {
                    if input.secret == "fail" {
                        return Err(Error::conflict("shop.echo_refused", "Refused."));
                    }
                    Ok(json!({ "length": input.secret.len() }))
                },
            )
            .for_any_signed_in(),
        ]
    }
}

const KEY: u128 = 77;

/// Danh sách nhật ký (ghi bất đồng bộ): hỏi lại tới khi có ít nhất `count` dòng khớp `query`.
async fn logged(app: &TestApp, query: &str, count: usize) -> Vec<Value> {
    for _ in 0..100 {
        let page = app
            .get(&format!("/api/v1/kernel/devtools/requests?{query}"))
            .token("admin")
            .send()
            .await
            .unwrap();
        assert_eq!(page.status(), StatusCode::OK, "{}", page.json());
        let rows = page.json()["data"].as_array().unwrap().clone();
        if rows.len() >= count {
            return rows;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    panic!("the log never got {count} rows for `{query}`");
}

#[tokio::test]
async fn request_log_khong_luu_than_va_token() {
    TestDatabase::run(async |test| {
        let robot = Principal::new(Uuid::from_u128(5), Vec::<String>::new())
            .via_api_key(Uuid::from_u128(KEY))
            .with_actor_kind(ActorKind::Agent);
        let app = TestApp::builder(test)
            .user("admin", 1, &["kernel.devtools.read"])
            .principal("robot-token-xyz", robot)
            .module(Shop)
            .build()
            .await
            .unwrap();
        let put = app
            .put("/api/v1/me/flags/hint.private?token=query-secret")
            .token("robot-token-xyz")
            .header("idempotency-key", "key-1")
            .json(json!({"value": "body-secret"}))
            .send()
            .await
            .unwrap();
        assert_eq!(put.status(), StatusCode::OK, "{}", put.json());
        let denied = app
            .get("/api/v1/kernel/devtools/requests")
            .token("robot-token-xyz")
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);

        let rows = logged(&app, "actor_kind=agent", 2).await;
        let text = serde_json::to_string(&rows).unwrap();
        for secret in ["query-secret", "body-secret", "robot-token-xyz", "hint.private"] {
            assert!(!text.contains(secret), "`{secret}` leaked into the log: {text}");
        }
        let flag = rows.iter().find(|row| row["method"] == "PUT").unwrap();
        assert_eq!(flag["path"], "/api/v1/me/flags/{key}");
        assert_eq!(flag["status"], 200);
        assert_eq!(flag["actor_kind"], "agent");
        assert_eq!(flag["api_key_id"], Uuid::from_u128(KEY).to_string());
        assert_eq!(flag["idempotency_key"], "key-1");
        assert!(flag["request_id"].is_string());
        let refused = rows.iter().find(|row| row["method"] == "GET").unwrap();
        assert_eq!(
            (refused["status"].as_u64(), refused["code"].as_str()),
            (Some(403), Some("permission_denied"))
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn lenh_mcp_ghi_dung_ten_tool_khong_ghi_tham_so() {
    TestDatabase::run(async |test| {
        let robot = Principal::new(Uuid::from_u128(5), Vec::<String>::new())
            .via_api_key(Uuid::from_u128(KEY))
            .with_actor_kind(ActorKind::Agent);
        let app = TestApp::builder(test)
            .user("admin", 1, &["kernel.devtools.read"])
            .principal("robot", robot)
            .module(Shop)
            .build()
            .await
            .unwrap();
        let client = app.mcp("robot").await.unwrap();
        client.call_tool(tool("shop.echo", json!({"secret": "tool-secret"}))).await.unwrap();
        let failed = client.call_tool(tool("shop.echo", json!({"secret": "fail"}))).await.unwrap();
        assert_eq!(failed.is_error, Some(true));
        client.cancel().await.unwrap();

        let rows = logged(&app, "path=mcp:shop.echo", 2).await;
        let text = serde_json::to_string(&rows).unwrap();
        assert!(!text.contains("tool-secret"), "tham số lọt vào nhật ký: {text}");
        assert!(rows.iter().all(|row| row["method"] == "MCP" && row["actor_kind"] == "agent"));
        let codes: Vec<Option<&str>> = rows.iter().map(|row| row["code"].as_str()).collect();
        assert_eq!(codes, [Some("shop.echo_refused"), None], "mới trước");
        // `POST /mcp` bao ngoài không ghi thêm dòng.
        let all = logged(&app, "actor_kind=agent", 2).await;
        assert!(all.iter().all(|row| row["path"] != "/mcp"), "{all:?}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn loc_nhat_ky_theo_actor_kind_va_key() {
    TestDatabase::run(async |test| {
        let robot = Principal::new(Uuid::from_u128(5), Vec::<String>::new())
            .via_api_key(Uuid::from_u128(KEY))
            .with_actor_kind(ActorKind::Agent);
        let service = Principal::new(Uuid::from_u128(6), Vec::<String>::new())
            .via_api_key(Uuid::from_u128(KEY + 1))
            .with_actor_kind(ActorKind::Service);
        let app = TestApp::builder(test)
            .user("admin", 1, &["kernel.devtools.read"])
            .principal("robot", robot)
            .principal("service", service)
            .build()
            .await
            .unwrap();
        for who in ["robot", "service", "service"] {
            app.get("/api/v1/me/flags").token(who).send().await.unwrap();
        }
        assert_eq!(logged(&app, "actor_kind=service", 2).await.len(), 2);
        let robot = logged(&app, &format!("api_key_id={}", Uuid::from_u128(KEY)), 1).await;
        assert_eq!(robot.len(), 1);
        assert_eq!(robot[0]["actor_kind"], "agent");
        let both = logged(&app, "actor_kind=agent,service&path=/api/v1/me/flags", 3).await;
        assert_eq!(both.len(), 3);
        let wrong = app
            .get("/api/v1/kernel/devtools/requests?actor_kind=robot")
            .token("admin")
            .send()
            .await
            .unwrap();
        assert_eq!(wrong.status(), StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}
