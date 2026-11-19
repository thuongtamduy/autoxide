//! Sao cấu hình vào sandbox và đẩy sang khách thật (bước 38, D-232, D-20261119-step38-sandbox-copy),
//! mọi module thật: cài đặt không bí mật, role, trang (cây, đang đăng) sang sandbox; trang đẩy về
//! thành bản nháp — trang đang đăng ở khách thật giữ nguyên, đẩy lại cùng nội dung → `unchanged`,
//! trùng slug → `conflict`, loại lạ → 422. Chạy trên cả SQLite và Postgres.

use autoxide_kernel::tenancy::{TenantContext, TenantId};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use sea_orm::{ConnectionTrait, Statement};
use serde_json::{Value, json};
use std::time::Duration;

const OWNER: [&str; 7] = [
    "notifications.templates.manage",
    "kernel.sandboxes.manage",
    "kernel.sandboxes.read",
    "kernel.settings.manage",
    "content.write",
    "content.publish",
    "auth.roles.manage",
];

async fn app(test: &TestDatabase) -> TestApp {
    let dir = std::env::temp_dir().join(format!("autoxide-sandbox-copy-{}", uuid::Uuid::now_v7()));
    let mut builder = TestApp::builder(test)
        .setting("AUTOXIDE_FILES_DIR", &dir.join("files").display().to_string())
        .setting("AUTOXIDE_MAIL_FILE_DIR", &dir.join("mail").display().to_string())
        .tenant("shop")
        .user_in("shop", "owner", 2, &OWNER);
    for module in crate::modules::all() {
        builder = builder.boxed_module(module);
    }
    builder.build().await.unwrap()
}

async fn call(
    app: &TestApp,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = app.request(method, &format!("/api/v1{path}")).token("owner");
    if let Some(body) = body {
        request = request.json(body);
    }
    let response = request.send().await.unwrap();
    let status = response.status();
    let body = if response.body().is_empty() { Value::Null } else { response.json() };
    (status, body)
}

async fn settle(app: &TestApp) {
    let worker = app.modules().worker(8).unwrap().blocked_delay(Duration::ZERO);
    let dispatcher = app.modules().dispatcher().unwrap();
    for _ in 0..5 {
        let ran = worker.run_once().await.unwrap();
        let delivered = dispatcher.run_once().await.unwrap();
        if ran == 0 && delivered == 0 {
            break;
        }
    }
}

/// Tạo trang `slug` (con của `parent`) rồi đăng; trả ID.
async fn page(app: &TestApp, slug: &str, parent: Option<&str>) -> String {
    let body =
        json!({"kind": "page", "title": slug.to_uppercase(), "slug": slug, "parent_id": parent});
    let (status, created) = call(app, Method::POST, "/content/entries", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["data"]["id"].as_str().unwrap().to_owned();
    let published = app
        .post(&format!("/api/v1/content/entries/{id}/publish"))
        .token("owner")
        .header("if-match", "*")
        .send()
        .await
        .unwrap();
    assert!(published.status().is_success(), "{}", published.text());
    id
}

/// Một cột của `content_entries` dòng `id` ở khách `tenant`.
async fn entry_text(tenant: &TenantContext, id: &str, column: &str) -> String {
    let read = tenant.database().read();
    let sql = format!("SELECT {column} AS v FROM content_entries WHERE id = $1");
    let row = read
        .query_one_raw(Statement::from_sql_and_values(
            read.get_database_backend(),
            &sql,
            [id.parse::<uuid::Uuid>().unwrap().into()],
        ))
        .await
        .unwrap()
        .unwrap();
    row.try_get("", "v").unwrap()
}

/// ID trang slug `slug` ở khách `tenant`.
async fn entry_by_slug(tenant: &TenantContext, slug: &str) -> Option<String> {
    let read = tenant.database().read();
    read.query_one_raw(Statement::from_sql_and_values(
        read.get_database_backend(),
        "SELECT id FROM content_entries WHERE tenant_id = $1 AND slug = $2",
        [tenant.id().as_uuid().into(), slug.into()],
    ))
    .await
    .unwrap()
    .map(|row| row.try_get::<uuid::Uuid>("", "id").unwrap().to_string())
}

async fn count(tenant: &TenantContext, sql: &str) -> i64 {
    let read = tenant.database().read();
    let row = read
        .query_one_raw(Statement::from_sql_and_values(
            read.get_database_backend(),
            sql,
            [tenant.id().as_uuid().into()],
        ))
        .await
        .unwrap()
        .unwrap();
    row.try_get("", "n").unwrap()
}

#[tokio::test]
async fn sao_cau_hinh_vao_sandbox_va_day_trang_thanh_ban_nhap() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let shop = app.tenant("shop").await.unwrap();

        // Cấu hình của khách thật: cài đặt, role, hai trang (cha – con) đang đăng.
        let (status, _) = call(
            &app,
            Method::PUT,
            "/kernel/settings/kernel.business_model",
            Some(json!({"value": "chain"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let role = json!({"name": "editor", "permissions": ["content.write"]});
        let (status, created) = call(&app, Method::POST, "/auth/roles", Some(role)).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let template = "/notifications/templates/notifications.job_finished/in_app/en";
        let draft =
            json!({"subject": "{{ label }} done", "body_text": "{{ label }}: {{ status }}"});
        let (status, saved) =
            call(&app, Method::PUT, &format!("{template}/draft"), Some(draft)).await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        let (status, saved) = call(&app, Method::POST, &format!("{template}/publish"), None).await;
        assert!(status.is_success(), "{saved}");
        let about = page(&app, "about", None).await;
        let team = page(&app, "team", Some(&about)).await;

        let path = format!("/kernel/tenants/{}/sandboxes", shop.id());
        let (status, created) = call(&app, Method::POST, &path, Some(json!({"name": "Try"}))).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let id = created["data"]["id"].as_str().unwrap().to_owned();
        settle(&app).await;
        let sandbox =
            app.registry().tenant(TenantId::from_uuid(id.parse().unwrap())).await.unwrap();

        // sao_cau_hinh_khong_sao_du_lieu_ca_nhan: cài đặt, role, trang có; người dùng thì không.
        let settings = "SELECT COUNT(*) AS n FROM kernel_tenant_settings WHERE tenant_id = $1 \
                        AND name = 'kernel.business_model' AND value = 'chain'";
        assert_eq!(count(&sandbox, settings).await, 1);
        let roles = "SELECT COUNT(*) AS n FROM auth_role WHERE tenant_id = $1 AND name = 'editor'";
        assert_eq!(count(&sandbox, roles).await, 1);
        let live = "SELECT COUNT(*) AS n FROM notifications_template_live WHERE tenant_id = $1";
        assert_eq!(count(&sandbox, live).await, 1, "mẫu đang dùng đã sao và xuất bản");
        let users = "SELECT COUNT(*) AS n FROM auth_user WHERE tenant_id = $1";
        assert_eq!(count(&sandbox, users).await, 0);
        let copied_about = entry_by_slug(&sandbox, "about").await.expect("trang about đã sao");
        let copied_team = entry_by_slug(&sandbox, "team").await.expect("trang con đã sao");
        assert_ne!(copied_about, about, "bản sao có ID mới");
        assert_eq!(entry_text(&sandbox, &copied_team, "status").await, "published");
        let parent = sandbox
            .database()
            .read()
            .query_one_raw(Statement::from_sql_and_values(
                sandbox.database().read().get_database_backend(),
                "SELECT parent_id FROM content_entries WHERE id = $1",
                [copied_team.parse::<uuid::Uuid>().unwrap().into()],
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get::<Option<uuid::Uuid>>("", "parent_id")
            .unwrap();
        assert_eq!(parent.map(|id| id.to_string()), Some(copied_about.clone()), "cây giữ nguyên");
        let pages = "SELECT COUNT(*) AS n FROM content_entries WHERE tenant_id = $1";
        assert_eq!(count(&sandbox, pages).await, 2);
        let _ = team;

        // Loại đẩy được.
        let (status, kinds) = call(&app, Method::GET, "/kernel/sandboxes/promotable", None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            kinds["data"].as_array().unwrap().iter().any(|kind| kind["kind"] == "content.page")
        );

        // Sửa trang trong sandbox (như người thử), rồi đẩy về.
        let write = sandbox.database().write();
        write
            .execute_raw(Statement::from_sql_and_values(
                write.get_database_backend(),
                "UPDATE content_entries SET title = $1 WHERE id = $2",
                ["About us (new)".into(), copied_about.parse::<uuid::Uuid>().unwrap().into()],
            ))
            .await
            .unwrap();
        let promote = format!("/kernel/sandboxes/{id}/promote");
        let items = json!({"items": [{"kind": "content.page", "id": copied_about}]});
        let (status, result) = call(&app, Method::POST, &promote, Some(items.clone())).await;
        assert_eq!(status, StatusCode::OK, "{result}");
        assert_eq!(result["data"]["items"][0]["outcome"], "updated_draft", "{result}");
        assert_eq!(result["data"]["items"][0]["target_id"], about);
        // dua_sang_khach_that_luon_la_ban_nhap: bản nháp đổi, trang vẫn đang đăng với bản cũ.
        assert_eq!(entry_text(&shop, &about, "title").await, "About us (new)");
        assert_eq!(entry_text(&shop, &about, "status").await, "published");
        let pending = "SELECT COUNT(*) AS n FROM content_entries WHERE tenant_id = $1 \
                       AND slug = 'about' AND has_unpublished_changes = true";
        assert_eq!(count(&shop, pending).await, 1, "bản đang đăng không bị ghi đè");
        // Đẩy lại cùng nội dung: không đổi.
        let (_, again) = call(&app, Method::POST, &promote, Some(items)).await;
        assert_eq!(again["data"]["items"][0]["outcome"], "unchanged", "{again}");

        // Trùng slug ở khách thật → `conflict` cho mục đó.
        page(&app, "contact", None).await;
        write
            .execute_raw(Statement::from_sql_and_values(
                write.get_database_backend(),
                "UPDATE content_entries SET slug = 'contact' WHERE id = $1",
                [copied_about.parse::<uuid::Uuid>().unwrap().into()],
            ))
            .await
            .unwrap();
        let items = json!({"items": [{"kind": "content.page", "id": copied_about}]});
        let (status, clash) = call(&app, Method::POST, &promote, Some(items)).await;
        assert_eq!(status, StatusCode::OK, "{clash}");
        assert_eq!(clash["data"]["items"][0]["outcome"], "conflict", "{clash}");
        assert_eq!(clash["data"]["items"][0]["conflict"]["field"], "slug");

        // Loại lạ → 422 với mã của từng mục.
        let items = json!({"items": [{"kind": "orders.order", "id": copied_about}]});
        let (status, refused) = call(&app, Method::POST, &promote, Some(items)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
        assert_eq!(refused["errors"][0]["code"], "tenancy.promote_kind_unknown", "{refused}");
    })
    .await
    .unwrap();
}
