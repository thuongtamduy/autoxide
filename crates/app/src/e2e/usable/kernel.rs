//! Phần của kernel dùng được qua API như quản trị viên thật (cả SQLite và Postgres): module, tag,
//! trường tuỳ biến, ghi chú trên bản ghi, lệnh của agent chờ duyệt bị từ chối, khách mới, xuất dữ
//! liệu, migration của placement, nhật ký request.

use autoxide_testkit::TestDatabase;
use serde_json::json;

use super::kit::{ADMIN_PASSWORD, call_tool, drain, expect, id, owner_of, real_app_with};

#[tokio::test]
async fn tag_truong_tuy_bien_ghi_chu_va_duyet_lenh_agent() {
    TestDatabase::run(async |test| {
        let real = real_app_with(test, &[], |builder| builder.multi_tenant()).await;
        let (app, admin) = (&real.app, real.admin.as_str());

        let modules = expect(app, "GET", "/api/v1/kernel/modules", admin, None, 200).await;
        assert!(modules["data"].as_array().unwrap().iter().any(|module| module["name"] == "content"));
        call_tool(app, admin, "kernel.list_modules", json!({})).await;
        call_tool(app, admin, "kernel.get_module", json!({"name": "content"})).await;

        let tag = expect(app, "POST", "/api/v1/kernel/tags", admin, Some(json!({"name": "VIP"})), 201).await;
        let tag = format!("/api/v1/kernel/tags/{}", id(&tag));
        let changed = expect(app, "PATCH", &tag, admin, Some(json!({"color": "red"})), 200).await;
        assert_eq!(changed["data"]["color"], "red");
        expect(app, "DELETE", &tag, admin, None, 204).await;

        let field = json!({"target": "content.entry", "key": "mood", "data_type": "text", "labels": {"en": "Mood"}});
        let field = expect(app, "POST", "/api/v1/kernel/custom-fields", admin, Some(field), 201).await;
        let field = format!("/api/v1/kernel/custom-fields/{}", id(&field));
        expect(app, "GET", &field, admin, None, 200).await;
        let changed = expect(app, "PATCH", &field, admin, Some(json!({"labels": {"en": "Feeling"}})), 200).await;
        assert_eq!(changed["data"]["labels"]["en"], "Feeling");

        let entry = expect(app, "POST", "/api/v1/content/entries", admin, Some(json!({"kind": "post", "title": "Hello"})), 201).await;
        let entry = id(&entry);
        let notes = format!("/api/v1/kernel/records/content.entry/{entry}/notes");
        expect(app, "POST", &notes, admin, Some(json!({"body": "Check the title"})), 201).await;
        let listed = expect(app, "GET", &notes, admin, None, 200).await;
        assert_eq!(listed["data"][0]["body"], "Check the title", "{listed}");

        // Agent (API key loại `agent`) xoá bài: chưa làm gì, thành yêu cầu duyệt; người duyệt từ chối.
        let key = json!({"name": "robot", "kind": "agent", "permissions": ["content.read", "content.delete"],
                         "current_password": ADMIN_PASSWORD});
        let key = expect(app, "POST", "/api/v1/auth/api-keys", admin, Some(key), 201).await;
        let secret = key["data"]["key"].as_str().unwrap().to_owned();
        let answer = app
            .delete(&format!("/api/v1/content/entries/{entry}"))
            .token(&secret)
            .header("idempotency-key", "agent-delete-1")
            .send()
            .await
            .unwrap();
        let pending = answer.json();
        assert_eq!(answer.status(), 202, "{pending}");
        let approval = id(&pending);
        let reason = json!({"reason": "Keep this post"});
        let rejected =
            expect(app, "POST", &format!("/api/v1/kernel/approvals/{approval}/reject"), admin, Some(reason), 200).await;
        assert_eq!(rejected["data"]["status"], "rejected");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khach_moi_xuat_du_lieu_migration_va_nhat_ky_request() {
    TestDatabase::run(async |test| {
        let real =
            real_app_with(test, &[], |builder| builder.multi_tenant().placement("spare")).await;
        let (app, admin) = (&real.app, real.admin.as_str());

        let body = json!({"name": "Shop B", "slug": "shop-b"});
        expect(app, "POST", "/api/v1/kernel/tenants", admin, Some(body), 201).await;
        call_tool(app, admin, "kernel.list_tenants", json!({})).await;
        call_tool(app, admin, "kernel.get_tenant", json!({"tenant": "shop-b"})).await;

        let export = call_tool(app, admin, "kernel.export_tenant", json!({})).await;
        let export_id = export["id"].as_str().unwrap_or_else(|| panic!("{export}")).to_owned();
        call_tool(app, admin, "kernel.get_tenant_export", json!({"export_id": export_id})).await;

        let run =
            call_tool(app, admin, "kernel.migrate_placements", json!({"dry_run": true})).await;
        if let Some(run_id) = run["id"].as_str() {
            call_tool(app, admin, "kernel.get_migration_run", json!({"id": run_id})).await;
        }
        call_tool(app, admin, "kernel.request_log", json!({})).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn sandbox_tao_gia_han_va_dua_ban_ghi_ve_khach_that_bang_tool() {
    TestDatabase::run(async |test| {
        let real = real_app_with(test, &[], |builder| builder.tenant("shop")).await;
        let app = &real.app;
        // Sandbox là của một khách thật, không của nền tảng: chủ khách `shop` làm.
        let owner = owner_of(app, "shop").await;
        let admin = owner.as_str();
        let sandbox = call_tool(app, admin, "kernel.create_sandbox", json!({"name": "Try"})).await;
        let sandbox_id = sandbox["id"].as_str().unwrap_or_else(|| panic!("{sandbox}")).to_owned();
        call_tool(app, admin, "kernel.list_sandboxes", json!({})).await;
        call_tool(app, admin, "kernel.extend_sandbox", json!({"id": sandbox_id})).await;
        let kinds = call_tool(app, admin, "kernel.list_promotable", json!({})).await;
        assert!(kinds.to_string().contains("content."), "{kinds}");

        // Job sao cấu hình và mở quyền vào sandbox chạy xong thì chủ khách mới vào được.
        drain(app).await;
        // Một trang viết trong sandbox (vào bằng header `Autoxide-Tenant`), rồi đưa về khách thật.
        let answer = app
            .post("/api/v1/content/entries")
            .token(admin)
            .header("autoxide-tenant", &sandbox_id)
            .json(json!({"kind": "page", "title": "Tried page"}))
            .send()
            .await
            .unwrap();
        let page = answer.json();
        assert_eq!(answer.status(), 201, "{page}");
        let item = format!("content.page:{}", id(&page));
        let promoted = call_tool(
            app,
            admin,
            "kernel.promote_sandbox",
            json!({"id": sandbox_id, "items": [item]}),
        )
        .await;
        assert!(promoted.to_string().contains("created_draft"), "{promoted}");

        // Trong sandbox: tin nhắn giữ lại thay vì gửi đi. Client MCP vào sandbox bằng header
        // `Autoxide-Tenant` như REST.
        let sandbox_header = axum::http::HeaderValue::from_str(&sandbox_id).unwrap();
        let router = app.router().layer(axum::middleware::map_request(
            move |mut request: axum::extract::Request| {
                let value = sandbox_header.clone();
                async move {
                    request.headers_mut().insert("autoxide-tenant", value);
                    request
                }
            },
        ));
        let client = autoxide_testkit::mcp::McpClient::new(router, "/mcp", Some(admin));
        let kept = client
            .call_tool(autoxide_testkit::tool("kernel.list_sandbox_messages", json!({})))
            .await
            .unwrap();
        assert_ne!(kept.is_error, Some(true), "{:?}", kept.content);
    })
    .await
    .unwrap();
}
