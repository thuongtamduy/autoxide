//! Yêu cầu duyệt của agent (bước 51 P12): yêu cầu chờ vào hộp việc của người quyết được, không vào
//! hộp của người khác; tool MCP liệt kê và quyết như REST.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_kernel::access::{ActorKind, Principal};
use autoxide_testkit::{TestApp, TestDatabase, tool};
use axum::http::StatusCode;
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
async fn yeu_cau_duyet_vao_hop_viec_va_tool_mcp() {
    TestDatabase::run(async |test| {
        let robot = Principal::new(Uuid::from_u128(9), ["kernel.views.share".to_owned()])
            .via_api_key(Uuid::from_u128(10))
            .with_actor_kind(ActorKind::Agent);
        let app = TestApp::builder(test)
            .user("boss", 1, &["kernel.views.share"])
            .user("other", 2, &[])
            .principal("robot", robot)
            .build()
            .await
            .unwrap();
        // Ghi thẳng một yêu cầu đang chờ (lệnh nào của module nào không quan trọng ở đây).
        use sea_orm::ConnectionTrait;
        let id = Uuid::now_v7();
        let database = app.database().write();
        let backend = database.get_database_backend();
        let tenant = autoxide_kernel::tenancy::DatabaseRegistry::single(app.database().clone())
            .default_tenant()
            .id()
            .as_uuid();
        let insert = sea_orm::sea_query::Query::insert()
            .into_table(sea_orm::sea_query::Alias::new("kernel_approval_requests"))
            .columns(
                [
                    "id",
                    "tenant_id",
                    "requested_by",
                    "api_key_id",
                    "summary",
                    "body_hash",
                    "permissions",
                    "status",
                    "expires_at",
                    "created_at",
                ]
                .map(sea_orm::sea_query::Alias::new),
            )
            .values_panic([
                id.into(),
                tenant.into(),
                Uuid::from_u128(9).into(),
                Uuid::from_u128(10).into(),
                "DELETE /api/v1/shop/things/1".into(),
                "h".into(),
                "[\"kernel.views.share\"]".into(),
                "pending".into(),
                (chrono::Utc::now() + chrono::Duration::hours(1)).into(),
                chrono::Utc::now().into(),
            ])
            .to_owned();
        database.execute_raw(backend.build(&insert)).await.unwrap();

        let work = |who: &'static str| {
            let app = &app;
            async move { app.get("/api/v1/kernel/work").token(who).send().await.unwrap().json() }
        };
        let boss = work("boss").await;
        let keys: Vec<&str> = boss["data"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["key"].as_str().unwrap())
            .collect();
        assert!(keys.contains(&format!("kernel.approvals:{id}").as_str()), "{boss}");
        let other = work("other").await;
        assert!(
            other["data"]["items"]
                .as_array()
                .unwrap()
                .iter()
                .all(|item| item["source"] != "kernel.approvals"),
            "người không quyết được không thấy: {other}"
        );

        let agent = app.mcp("robot").await.unwrap();
        let listed = agent
            .call_tool(tool("kernel.list_approvals", json!({"status": "pending"})))
            .await
            .unwrap();
        assert_eq!(listed.structured_content.unwrap()["data"][0]["id"], id.to_string());
        let refused = agent
            .call_tool(tool("kernel.decide_approval", json!({"id": id, "decision": "approve"})))
            .await
            .unwrap();
        assert_eq!(refused.structured_content.unwrap()["code"], "kernel.approval_forbidden");
        agent.cancel().await.unwrap();

        let person = app.mcp("boss").await.unwrap();
        let rejected = person
            .call_tool(tool(
                "kernel.decide_approval",
                json!({"id": id, "decision": "reject", "reason": "Không"}),
            ))
            .await
            .unwrap();
        assert_eq!(rejected.structured_content.unwrap()["data"]["status"], "rejected");
        person.cancel().await.unwrap();
        let after = work("boss").await;
        assert!(
            after["data"]["items"]
                .as_array()
                .unwrap()
                .iter()
                .all(|item| item["source"] != "kernel.approvals")
        );
        assert_eq!(
            app.get("/api/v1/kernel/approvals?status=rejected")
                .token("robot")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
    })
    .await
    .unwrap();
}
