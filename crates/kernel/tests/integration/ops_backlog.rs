//! Tồn đọng qua REST và MCP (bước 39, A3): `GET /api/v1/kernel/ops/backlog` và tool
//! `kernel.ops_backlog` trả cùng số liệu, chỉ cho nhân sự khách nền tảng có `kernel.ops.read`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_testkit::{TestApp, TestDatabase, tool};
use axum::http::StatusCode;
use serde_json::json;

#[tokio::test]
async fn ops_backlog_can_permission() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test)
            .tenant("shop")
            .user("ops", 1, &["kernel.ops.read"])
            .user("nobody", 2, &[])
            .user_in("shop", "owner", 3, &["kernel.ops.read"])
            .build()
            .await
            .unwrap();

        let answer = app.get("/api/v1/kernel/ops/backlog").token("ops").send().await.unwrap();
        assert_eq!(answer.status(), StatusCode::OK, "{}", answer.json());
        let body = answer.json();
        let main = &body["data"]["databases"][0];
        assert_eq!(main["database"], "main");
        // Module `kernel` luôn có hàng đợi job và bảng event.
        let kernel = main["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|module| module["module"] == "kernel")
            .unwrap();
        assert!(kernel["jobs_pending"].is_u64(), "{kernel}");
        assert_eq!(main["pools"].as_array().unwrap().len(), 2);
        assert!(main["last_backup_at"].is_null());

        // Thiếu permission: 403. Quản trị của một khách (không phải nền tảng): 403 — số liệu là của
        // cả database, gồm khách khác.
        let refused = app.get("/api/v1/kernel/ops/backlog").token("nobody").send().await.unwrap();
        assert_eq!(refused.status(), StatusCode::FORBIDDEN);
        let shop = app
            .get("/api/v1/kernel/ops/backlog")
            .token("owner")
            .header("autoxide-tenant", "shop")
            .send()
            .await
            .unwrap();
        assert_eq!(shop.status(), StatusCode::FORBIDDEN, "{}", shop.json());

        // MCP: cùng service, cùng kết quả.
        let agent = app.mcp("ops").await.unwrap();
        let listed = agent.call_tool(tool("kernel.ops_backlog", json!({}))).await.unwrap();
        let data = listed.structured_content.unwrap();
        assert_eq!(data["data"]["databases"][0]["database"], "main", "{data}");
        agent.cancel().await.unwrap();
        // Không có permission thì không thấy tool (và không gọi được).
        let stranger = app.mcp("nobody").await.unwrap();
        let tools = stranger.list_all_tools().await.unwrap();
        assert!(tools.iter().all(|tool| tool.name != "kernel.ops_backlog"));
        stranger.cancel().await.unwrap();
    })
    .await
    .unwrap();
}
