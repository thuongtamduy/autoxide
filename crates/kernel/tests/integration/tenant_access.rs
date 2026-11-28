//! Đối chiếu thiết kế 2 của bước 33: khách tạm khoá (chặn mọi route, kể cả đọc) khác khách chỉ đọc
//! (đọc được, lệnh ghi qua REST và tool MCP ghi bị chặn, route đánh dấu như xuất dữ liệu vẫn chạy)
//! — A-3.11; danh sách khách có lọc và `expand` — A-c7; nhân sự nền tảng đặt cấu hình của một khách
//! — A-b7. Chạy trên cả SQLite và Postgres.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_kernel::access::Principal;
use autoxide_kernel::module::Module;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, TenantContext, allowed_while_read_only};
use autoxide_testkit::{TestApp, TestDatabase, tool};
use axum::http::StatusCode;
use serde_json::{Value, json};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

/// Module thử: một lệnh ghi được đánh dấu chạy cả khi khách chỉ đọc (như xuất dữ liệu), một lệnh
/// ghi thường.
struct Demo;

/// Export the data of the tenant (test).
#[utoipa::path(
    post,
    path = "/export",
    extensions(("x-permissions" = json!([]))),
    security(("bearer" = [])),
    responses((status = 204, description = "Exported."))
)]
async fn export(_tenant: TenantContext, _principal: Principal) -> StatusCode {
    StatusCode::NO_CONTENT
}

/// Change something (test).
#[utoipa::path(
    post,
    path = "/change",
    extensions(("x-permissions" = json!([]))),
    security(("bearer" = [])),
    responses((status = 204, description = "Changed."))
)]
async fn change(_tenant: TenantContext, _principal: Principal) -> StatusCode {
    StatusCode::NO_CONTENT
}

impl Module for Demo {
    fn name(&self) -> &'static str {
        "demo"
    }

    fn routes(&self) -> OpenApiRouter {
        OpenApiRouter::new()
            .routes(routes!(export))
            .layer(allowed_while_read_only())
            .routes(routes!(change))
    }
}

/// Module thử khai bằng manifest + `#[endpoint]`: `Routes::allowed_while_read_only` đánh dấu route như lớp cũ.
struct Marked;

/// Confirm something from a mail link (test).
#[autoxide_kernel::http::endpoint(
    post,
    path = "/confirm",
    signed_in,
    responses((status = 204, description = "Confirmed."))
)]
async fn confirm(
    _tenant: TenantContext,
    _principal: Principal,
) -> autoxide_kernel::http::NoContent {
    autoxide_kernel::http::NoContent
}

/// Edit something (test).
#[autoxide_kernel::http::endpoint(
    post,
    path = "/edit",
    signed_in,
    responses((status = 204, description = "Edited."))
)]
async fn edit(_tenant: TenantContext, _principal: Principal) -> autoxide_kernel::http::NoContent {
    autoxide_kernel::http::NoContent
}

impl Module for Marked {
    fn manifest(&self) -> autoxide_kernel::module::Manifest {
        autoxide_kernel::module::Manifest::new(
            "marked",
            "Test module with a route allowed while read-only.",
        )
    }

    fn start(
        &mut self,
        context: &mut autoxide_kernel::module::StartContext<'_>,
    ) -> Result<(), autoxide_kernel::error::Error> {
        context.routes(
            autoxide_kernel::routes![confirm]
                .allowed_while_read_only()
                .merge(autoxide_kernel::routes![edit]),
        )
    }
}

const STAFF: [&str; 2] = ["kernel.domains.manage", "kernel.settings.manage"];

async fn app(test: &TestDatabase) -> TestApp {
    TestApp::builder(test)
        .tenant("acme")
        .tenant("beta")
        // `kernel.public_url` của khách nhiều-khách phải là tên miền đã xác minh (D-20261125-tenant-site-verified).
        .domain("acme", "acme.example")
        .user("platform", 1, &["kernel.tenants.manage", "kernel.tenants.read"])
        .user_in("acme", "acme", 2, &STAFF)
        .user_in("beta", "beta", 3, &STAFF)
        .module(Demo)
        .module(Marked)
        .build()
        .await
        .unwrap()
}

/// `PATCH /kernel/tenants/{id}` của nhân sự nền tảng.
async fn change_tenant(app: &TestApp, slug: &str, changes: Value) -> (StatusCode, Value) {
    let id = app.tenant(slug).await.unwrap().id();
    let answer = app
        .patch(&format!("/api/v1/kernel/tenants/{id}"))
        .token("platform")
        .json(changes)
        .send()
        .await
        .unwrap();
    (answer.status(), answer.json())
}

async fn call(app: &TestApp, method: &str, path: &str, token: &str) -> (StatusCode, Value) {
    let request = match method {
        "GET" => app.get(path),
        "POST" => app.post(path).json(json!({"domain": "shop.example"})),
        "PUT" => app.put(path).json(json!({"value": "https://acme.example"})),
        _ => unreachable!(),
    };
    let answer = request.token(token).send().await.unwrap();
    (answer.status(), answer.json())
}

fn code(answer: &(StatusCode, Value)) -> (StatusCode, Option<&str>) {
    (answer.0, answer.1["code"].as_str())
}

#[tokio::test]
async fn khach_chi_doc_doc_duoc_khong_ghi_duoc() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let (status, body) = change_tenant(&app, "acme", json!({"read_only": true})).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            (body["data"]["read_only"].clone(), body["data"]["status"].clone()),
            (json!(true), json!("active"))
        );

        // Đọc được.
        let current = call(&app, "GET", "/api/v1/kernel/tenant", "acme").await;
        assert_eq!(
            (current.0, current.1["data"]["read_only"].clone()),
            (StatusCode::OK, json!(true))
        );
        assert_eq!(call(&app, "GET", "/api/v1/kernel/domains", "acme").await.0, StatusCode::OK);
        // Ghi bị chặn, cùng một lỗi cho mọi route.
        let read_only = (StatusCode::FORBIDDEN, Some("tenant_read_only"));
        assert_eq!(code(&call(&app, "POST", "/api/v1/kernel/domains", "acme").await), read_only);
        let setting = "/api/v1/kernel/settings/kernel.public_url";
        assert_eq!(code(&call(&app, "PUT", setting, "acme").await), read_only);
        assert_eq!(code(&call(&app, "POST", "/api/v1/demo/change", "acme").await), read_only);
        assert_eq!(code(&call(&app, "POST", "/api/v1/marked/edit", "acme").await), read_only);
        assert_eq!(
            call(&app, "POST", "/api/v1/marked/confirm", "acme").await.0,
            StatusCode::NO_CONTENT
        );
        // Route đánh dấu (xuất dữ liệu) vẫn chạy; khách khác không bị ảnh hưởng.
        assert_eq!(
            call(&app, "POST", "/api/v1/demo/export", "acme").await.0,
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            call(&app, "POST", "/api/v1/demo/change", "beta").await.0,
            StatusCode::NO_CONTENT
        );

        // MCP: tool đọc chạy, tool ghi nhận cùng lỗi.
        let client = app.mcp("acme").await.unwrap();
        let listed = client.call_tool(tool("kernel.list_domains", json!({}))).await.unwrap();
        assert_ne!(listed.is_error, Some(true), "{:?}", listed.structured_content);
        let added = client
            .call_tool(tool("kernel.add_domain", json!({"domain": "shop.example"})))
            .await
            .unwrap();
        assert_eq!(added.is_error, Some(true));
        assert_eq!(added.structured_content.unwrap()["code"], "tenant_read_only");
        client.cancel().await.unwrap();

        // Khách nền tảng không đặt chỉ đọc được.
        let platform = app
            .patch(&format!("/api/v1/kernel/tenants/{DEFAULT_TENANT}"))
            .token("platform")
            .json(json!({"read_only": true}))
            .send()
            .await
            .unwrap();
        assert_eq!(platform.status(), StatusCode::UNPROCESSABLE_ENTITY, "{}", platform.json());
        // Gỡ chỉ đọc: ghi lại được.
        let (status, _) = change_tenant(&app, "acme", json!({"read_only": false})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            call(&app, "POST", "/api/v1/demo/change", "acme").await.0,
            StatusCode::NO_CONTENT
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khach_tam_khoa_khong_doc_duoc() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let (status, body) = change_tenant(&app, "acme", json!({"status": "suspended"})).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let suspended = (StatusCode::FORBIDDEN, Some("tenant_suspended"));
        assert_eq!(code(&call(&app, "GET", "/api/v1/kernel/tenant", "acme").await), suspended);
        assert_eq!(code(&call(&app, "GET", "/api/v1/kernel/domains", "acme").await), suspended);
        assert_eq!(code(&call(&app, "POST", "/api/v1/demo/export", "acme").await), suspended);
        // MCP: không vào được, kể cả tool chỉ đọc.
        assert!(app.mcp("acme").await.is_err());

        // Hai cờ độc lập: thêm chỉ đọc khi đang khoá vẫn chặn đọc; mở khoá thì chỉ còn chặn ghi.
        let (status, _) = change_tenant(&app, "acme", json!({"read_only": true})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(code(&call(&app, "GET", "/api/v1/kernel/domains", "acme").await), suspended);
        let (status, body) = change_tenant(&app, "acme", json!({"status": "active"})).await;
        assert_eq!((status, body["data"]["read_only"].clone()), (StatusCode::OK, json!(true)));
        assert_eq!(call(&app, "GET", "/api/v1/kernel/domains", "acme").await.0, StatusCode::OK);
        let read_only = (StatusCode::FORBIDDEN, Some("tenant_read_only"));
        assert_eq!(code(&call(&app, "POST", "/api/v1/demo/change", "acme").await), read_only);
    })
    .await
    .unwrap();
}

/// Slug của các khách trong trang `GET /kernel/tenants?<query>`, theo thứ tự tăng dần.
async fn slugs(app: &TestApp, query: &str) -> Vec<String> {
    let answer = app
        .get(&format!("/api/v1/kernel/tenants?order=asc&{query}"))
        .token("platform")
        .send()
        .await
        .unwrap();
    assert_eq!(answer.status(), StatusCode::OK, "{query}: {}", answer.json());
    let body = answer.json();
    body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["slug"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn danh_sach_khach_loc_va_expand() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        change_tenant(&app, "acme", json!({"read_only": true})).await;
        let all = ["default", "acme", "beta"].map(str::to_owned).to_vec();
        assert_eq!(slugs(&app, "").await, all);
        assert_eq!(slugs(&app, "read_only=true").await, ["acme"]);
        assert_eq!(slugs(&app, "read_only=false").await, ["default", "beta"]);
        assert_eq!(slugs(&app, "placement_kind=shared&schema_state=ready").await, all);
        assert!(slugs(&app, "placement_kind=dedicated").await.is_empty());
        assert!(slugs(&app, "schema_state=behind").await.is_empty());
        // Chưa có module thuê bao (bước 35): không khách nào có thuê bao hay gần hạn mức.
        assert!(slugs(&app, "subscription_status=past_due").await.is_empty());
        assert!(slugs(&app, "near_limit=true").await.is_empty());
        assert_eq!(slugs(&app, "near_limit=false").await, all);

        // `expand`: placement của cả trang; thuê bao, hạn mức `null`. Không `expand`: không có khoá.
        let page = app
            .get("/api/v1/kernel/tenants?expand=placement,subscription,usage")
            .token("platform")
            .send()
            .await
            .unwrap()
            .json();
        for tenant in page["data"].as_array().unwrap() {
            let expanded = &tenant["expanded"];
            assert_eq!(expanded["placement"]["name"], "main", "{tenant}");
            assert_eq!(expanded["placement"]["kind"], "shared");
            assert_eq!(
                (expanded["subscription"].clone(), expanded["usage"].clone()),
                (Value::Null, Value::Null)
            );
        }
        let plain = app.get("/api/v1/kernel/tenants").token("platform").send().await.unwrap();
        assert!(plain.json()["data"][0].get("expanded").is_none());

        // Giá trị sai: báo mọi ô một lần.
        let refused = app
            .get(
                "/api/v1/kernel/tenants?expand=owner&placement_kind=cloud&subscription_status=late",
            )
            .token("platform")
            .send()
            .await
            .unwrap();
        assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let fields: Vec<String> = refused.json()["errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|error| error["field"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(fields, ["placement_kind", "subscription_status", "expand"]);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nhan_su_nen_tang_dat_cau_hinh_cua_khach() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        // Khách chỉ đọc: nhân sự nền tảng vẫn đặt được (việc của nền tảng, không phải của khách).
        change_tenant(&app, "acme", json!({"read_only": true})).await;
        let path = "/api/v1/kernel/tenants/acme/settings/kernel.public_url";
        let set = call(&app, "PUT", path, "platform").await;
        assert_eq!(set.0, StatusCode::OK, "{}", set.1);
        assert_eq!(set.1["data"]["value"], "https://acme.example");
        // Khách thấy giá trị (đọc qua endpoint của khách); khách khác không.
        let listed = call(&app, "GET", "/api/v1/kernel/settings", "acme").await;
        let public_url = |body: &Value| {
            body["data"]
                .as_array()
                .unwrap()
                .iter()
                .find(|setting| setting["name"] == "kernel.public_url")
                .unwrap()["value"]
                .clone()
        };
        assert_eq!(public_url(&listed.1), json!("https://acme.example"));
        let theirs = call(&app, "GET", "/api/v1/kernel/settings", "beta").await;
        assert_eq!(public_url(&theirs.1), Value::Null);
        let by_platform =
            call(&app, "GET", "/api/v1/kernel/tenants/acme/settings", "platform").await;
        assert_eq!(public_url(&by_platform.1), json!("https://acme.example"));

        // Chỉ nhân sự nền tảng; khách không có thì 404.
        let refused =
            call(&app, "PUT", "/api/v1/kernel/tenants/beta/settings/kernel.public_url", "acme")
                .await;
        assert_eq!(refused.0, StatusCode::FORBIDDEN, "{}", refused.1);
        let missing = call(
            &app,
            "PUT",
            "/api/v1/kernel/tenants/nobody/settings/kernel.public_url",
            "platform",
        )
        .await;
        assert_eq!(code(&missing), (StatusCode::NOT_FOUND, Some("tenant_not_found")));
        let unknown =
            call(&app, "PUT", "/api/v1/kernel/tenants/acme/settings/kernel.nothing", "platform")
                .await;
        assert_eq!(
            code(&unknown),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("tenant_setting_unknown"))
        );

        // Bỏ: về cấu hình chung.
        let removed = app.delete(path).token("platform").send().await.unwrap();
        assert_eq!(removed.status(), StatusCode::NO_CONTENT);
        let listed = call(&app, "GET", "/api/v1/kernel/tenants/acme/settings", "platform").await;
        assert_eq!(public_url(&listed.1), Value::Null);
    })
    .await
    .unwrap();
}
