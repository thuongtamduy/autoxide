//! Yêu cầu duyệt của agent (bước 51 P12): yêu cầu chờ vào hộp việc của người quyết được, không vào
//! hộp của người khác; tool MCP liệt kê và quyết như REST.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_kernel::access::{ActorKind, Principal};
use autoxide_kernel::time::Timestamp;
use autoxide_testkit::{TestApp, TestDatabase, tool};
use axum::http::StatusCode;
use jiff::SignedDuration;
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
                (Timestamp::now() + SignedDuration::from_hours(1)).into(),
                Timestamp::now().into(),
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

/// Ghi thẳng một yêu cầu duyệt của `requested_by` cần `permission` (rỗng: lệnh không đòi permission nào), trạng
/// thái `status`.
async fn insert_request(
    app: &TestApp,
    requested_by: u128,
    permission: &str,
    status: &str,
) -> String {
    use sea_orm::ConnectionTrait;
    use sea_orm::sea_query::{Alias, Query};
    let id = Uuid::now_v7();
    let tenant = autoxide_kernel::tenancy::DatabaseRegistry::single(app.database().clone())
        .default_tenant()
        .id()
        .as_uuid();
    let now = Timestamp::now();
    let insert = Query::insert()
        .into_table(Alias::new("kernel_approval_requests"))
        .columns(
            [
                "id",
                "tenant_id",
                "requested_by",
                "summary",
                "body_hash",
                "permissions",
                "status",
                "expires_at",
                "created_at",
            ]
            .map(Alias::new),
        )
        .values_panic([
            id.into(),
            tenant.into(),
            Uuid::from_u128(requested_by).into(),
            "DELETE /api/v1/shop/things/1".into(),
            "h".into(),
            if permission.is_empty() { "[]".to_owned() } else { format!("[\"{permission}\"]") }
                .into(),
            status.into(),
            (now + SignedDuration::from_hours(1)).into(),
            now.into(),
        ])
        .to_owned();
    app.database().write().execute(&insert).await.unwrap();
    id.to_string()
}

// Danh sách yêu cầu duyệt phân trang trong database: người duyệt chỉ thấy yêu cầu cần permission mình có (lọc
// trong SQL, không thấy của người khác), theo `next_cursor` tới hết không trùng / sót; trạng thái sai và tham số
// trang sai báo chung một lỗi 422.
#[tokio::test]
async fn yeu_cau_duyet_phan_trang_chi_cai_duoc_xem() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test)
            .user("boss", 1, &["kernel.views.share"])
            .user("other", 2, &[])
            .build()
            .await
            .unwrap();
        let mut visible = Vec::new();
        for index in 0..7 {
            // Xen kẽ: yêu cầu cần permission `boss` có, và yêu cầu cần permission khác (của người khác).
            visible.push(insert_request(&app, 9, "kernel.views.share", "pending").await);
            insert_request(
                &app,
                8,
                "shop_x.delete",
                if index % 2 == 0 { "pending" } else { "rejected" },
            )
            .await;
        }
        insert_request(&app, 9, "kernel.views.share", "rejected").await;
        let walk = async |token: &str, query: &str| -> Vec<String> {
            let mut seen = Vec::new();
            let mut cursor: Option<String> = None;
            loop {
                let mut path = format!("/api/v1/kernel/approvals?limit=3&{query}");
                if let Some(cursor) = &cursor {
                    path.push_str(&format!("&cursor={cursor}"));
                }
                let page = app.get(&path).token(token).send().await.unwrap().json();
                seen.extend(
                    page["data"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|a| a["id"].as_str().unwrap().to_owned()),
                );
                match page["pagination"]["next_cursor"].as_str() {
                    Some(next) => cursor = Some(next.to_owned()),
                    None => return seen,
                }
            }
        };
        let mut newest_first = visible.clone();
        newest_first.reverse();
        assert_eq!(walk("boss", "status=pending").await, newest_first);
        assert_eq!(walk("boss", "status=pending&order=asc").await, visible);
        assert_eq!(walk("boss", "").await.len(), 8, "kể cả yêu cầu đã từ chối");
        assert!(
            walk("other", "").await.is_empty(),
            "không quyết được, không phải của mình: không thấy"
        );
        let counted = app
            .get("/api/v1/kernel/approvals?status=pending&count=true")
            .token("boss")
            .send()
            .await
            .unwrap()
            .json();
        assert_eq!(counted["pagination"]["total"], 7);

        let wrong = app
            .get("/api/v1/kernel/approvals?status=done&limit=0")
            .token("boss")
            .send()
            .await
            .unwrap();
        assert_eq!(wrong.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let fields: Vec<String> = wrong.json()["errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|error| error["field"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(fields, ["status", "limit"]);
    })
    .await
    .unwrap();
}

// Tên permission có `_` (ký tự đại diện của `LIKE`): bộ lọc SQL thoát nó, người duyệt vẫn thấy yêu cầu của mình;
// tên chỉ khác ở chỗ `_` không lọt vào.
#[tokio::test]
async fn nguoi_duyet_co_permission_chua_gach_duoi_thay_yeu_cau() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test)
            .user("fields", 1, &["kernel.custom_fields.manage"])
            .build()
            .await
            .unwrap();
        let mut visible = Vec::new();
        for _ in 0..3 {
            visible.push(insert_request(&app, 9, "kernel.custom_fields.manage", "pending").await);
        }
        insert_request(&app, 9, "kernel.customXfields.manage", "pending").await;
        visible.reverse();
        let page = app
            .get("/api/v1/kernel/approvals?status=pending&count=true")
            .token("fields")
            .send()
            .await
            .unwrap()
            .json();
        let ids: Vec<String> = page["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|approval| approval["id"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(ids, visible, "{page}");
        assert_eq!(page["pagination"]["total"], 3);
    })
    .await
    .unwrap();
}

// Lệnh không đòi permission (việc của chính người gọi, vd huỷ đơn của mình): chính chủ của agent duyệt trong phiên
// đăng nhập của họ; người khác — kể cả có mọi quyền — không duyệt thay, agent không tự duyệt
// (D-20261128-agent-approval-audit).
#[tokio::test]
async fn lenh_khong_permission_do_chinh_chu_duyet() {
    TestDatabase::run(async |test| {
        let robot = Principal::new(Uuid::from_u128(1), Vec::<String>::new())
            .via_api_key(Uuid::from_u128(10))
            .with_actor_kind(ActorKind::Agent);
        let app = TestApp::builder(test)
            .user("owner", 1, &[])
            .user("other", 2, &["kernel.views.share"])
            .principal("robot", robot)
            .build()
            .await
            .unwrap();
        let first = insert_request(&app, 1, "", "pending").await;
        let decide = |who: &'static str, id: String| {
            let app = &app;
            async move {
                app.post(&format!("/api/v1/kernel/approvals/{id}/approve"))
                    .token(who)
                    .send()
                    .await
                    .unwrap()
            }
        };
        let other = decide("other", first.clone()).await;
        assert_ne!(other.status(), StatusCode::OK, "người khác không duyệt thay chính chủ");
        let robot = decide("robot", first.clone()).await;
        assert_ne!(robot.status(), StatusCode::OK, "agent không tự duyệt");
        let owner = decide("owner", first.clone()).await;
        assert_eq!(owner.status(), StatusCode::OK);
        assert_eq!(owner.json()["data"]["status"], "approved");
    })
    .await
    .unwrap();
}

// Lệnh xoá của kernel mang `x-approval` (D-20261128-agent-approval-audit): agent gọi thì chưa có gì chạy, nhận 202
// và yêu cầu duyệt; người gọi cùng lệnh thì chạy thẳng tới handler.
#[tokio::test]
async fn agent_xoa_tag_phai_cho_duyet() {
    TestDatabase::run(async |test| {
        let robot = Principal::new(Uuid::from_u128(1), ["kernel.tags.manage".to_owned()])
            .via_api_key(Uuid::from_u128(10))
            .with_actor_kind(ActorKind::Agent);
        let app = TestApp::builder(test)
            .user("owner", 1, &["kernel.tags.manage"])
            .principal("robot", robot)
            .build()
            .await
            .unwrap();
        let path = format!("/api/v1/kernel/tags/{}", Uuid::now_v7());
        let agent = app.delete(&path).token("robot").send().await.unwrap();
        assert_eq!(agent.status(), StatusCode::ACCEPTED);
        let body = agent.json();
        assert_eq!(body["data"]["status"], "pending", "{body}");
        assert_eq!(body["data"]["permissions"], json!(["kernel.tags.manage"]));
        let person = app.delete(&path).token("owner").send().await.unwrap();
        assert_ne!(person.status(), StatusCode::ACCEPTED);
    })
    .await
    .unwrap();
}
