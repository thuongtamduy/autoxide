//! Tên miền riêng của khách (bước 33 C1, D-190): thêm → bản ghi TXT phải đặt; xác minh bằng DNS giả;
//! `GET /domains/allowed` cho Caddy; tên miền sai / của khách khác; thu hồi; request theo `Host`;
//! job `kernel.verify_domains`; khách khác không đụng được tên miền của mình. Chạy trên cả SQLite và
//! Postgres.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_kernel::error::Error;
use autoxide_kernel::module::Module;
use autoxide_kernel::response::Data;
use autoxide_kernel::tenancy::{TenantContext, TenantDomains};
use autoxide_testkit::{TestApp, TestDatabase, tool};
use axum::http::StatusCode;
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Expr, Query};
use serde_json::{Value, json};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

const MANAGE: &str = "kernel.domains.manage";

/// Module thử: route công khai trả khách của request.
struct Probe;

/// Tell which tenant the request reached.
#[utoipa::path(
    get,
    path = "/",
    extensions(("x-public" = json!(true))),
    responses((status = 200, description = "The tenant of the request.", body = Data<Value>))
)]
async fn whoami(tenant: TenantContext) -> Result<Data<Value>, Error> {
    Ok(Data { data: json!({"tenant": tenant.slug()}) })
}

impl Module for Probe {
    fn name(&self) -> &'static str {
        "probe"
    }

    fn routes(&self) -> OpenApiRouter {
        OpenApiRouter::new().routes(routes!(whoami))
    }
}

async fn app(test: &TestDatabase) -> TestApp {
    TestApp::builder(test)
        .setting("AUTOXIDE_HTTP_PUBLIC_URL", "https://api.platform.test")
        .tenant("a")
        .tenant("b")
        .user_in("a", "admin-a", 1, &[MANAGE])
        .user_in("b", "admin-b", 2, &[MANAGE])
        .user_in("a", "reader-a", 3, &[])
        .module(Probe)
        .build()
        .await
        .unwrap()
}

async fn add(app: &TestApp, token: &str, domain: &str) -> (StatusCode, Value) {
    let answer = app
        .post("/api/v1/kernel/domains")
        .token(token)
        .json(json!({"domain": domain}))
        .send()
        .await
        .unwrap();
    (answer.status(), answer.json())
}

async fn verify(app: &TestApp, token: &str, id: &str) -> (StatusCode, Value) {
    let answer =
        app.post(&format!("/api/v1/kernel/domains/{id}/verify")).token(token).send().await.unwrap();
    (answer.status(), answer.json())
}

async fn allowed(app: &TestApp, domain: &str) -> StatusCode {
    app.get(&format!("/api/v1/kernel/domains/allowed?domain={domain}"))
        .send()
        .await
        .unwrap()
        .status()
}

/// Tên các event tên miền đã ghi trong database của khách `tenant`.
async fn events(tenant: &TenantContext) -> Vec<String> {
    use sea_orm::sea_query::ExprTrait as _;
    let select = Query::select()
        .column(Alias::new("name"))
        .from(Alias::new("kernel_events"))
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
        .and_where(Expr::col(Alias::new("name")).like("kernel.domain_%"))
        .order_by(Alias::new("occurred_at"), sea_orm::sea_query::Order::Asc)
        .order_by(Alias::new("id"), sea_orm::sea_query::Order::Asc)
        .to_owned();
    let rows = tenant.database().read().query_all(&select).await.unwrap();
    rows.iter().map(|row| row.try_get::<String>("", "name").unwrap()).collect()
}

#[tokio::test]
async fn them_xac_minh_cho_phep_va_thu_hoi() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let (status, body) = add(&app, "admin-a", " Shop-A.Test. ").await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let domain = &body["data"];
        let id = domain["id"].as_str().unwrap().to_owned();
        assert_eq!(domain["domain"], "shop-a.test");
        assert_eq!(domain["status"], "pending");
        assert_eq!(domain["record"]["type"], "TXT");
        assert_eq!(domain["record"]["name"], "_autoxide-challenge.shop-a.test");
        let value = domain["record"]["value"].as_str().unwrap().to_owned();
        assert!(value.starts_with("autoxide-verify="), "{value}");
        // Gửi lại: chính tên miền đó, không đổi token.
        let (status, again) = add(&app, "admin-a", "shop-a.test").await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(
            (again["data"]["id"].as_str(), &again["data"]["record"]["value"]),
            (Some(id.as_str()), &json!(value))
        );
        assert_eq!(allowed(&app, "shop-a.test").await, StatusCode::NOT_FOUND);

        // Chưa có bản ghi: vẫn `pending`, nói rõ vì sao.
        let (status, body) = verify(&app, "admin-a", &id).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["data"]["status"], "pending");
        assert!(body["data"]["check_error"].as_str().unwrap().contains("No TXT record"), "{body}");
        assert!(body["data"]["last_checked_at"].is_string());
        // Giá trị sai: vẫn `pending`.
        app.dns().set_txt("_autoxide-challenge.shop-a.test", &["autoxide-verify=wrong"]);
        let (_, body) = verify(&app, "admin-a", &id).await;
        assert_eq!(body["data"]["status"], "pending");
        assert!(body["data"]["check_error"].as_str().unwrap().contains("expected value"));
        // DNS hỏng: vẫn `pending`, lỗi ghi lại.
        app.dns().fail(true);
        let (status, body) = verify(&app, "admin-a", &id).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body["data"]["check_error"].as_str().unwrap().starts_with("DNS lookup failed"));
        app.dns().fail(false);
        // Giá trị đúng (kèm bản ghi khác cùng tên): `verified`.
        app.dns().set_txt("_autoxide-challenge.shop-a.test", &["v=spf1 -all", &value]);
        let (status, body) = verify(&app, "admin-a", &id).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["data"]["status"], "verified");
        assert!(body["data"]["verified_at"].is_string());
        assert!(body["data"]["check_error"].is_null());
        assert_eq!(allowed(&app, "shop-a.test").await, StatusCode::OK);
        assert_eq!(allowed(&app, "SHOP-A.test.").await, StatusCode::OK);
        assert_eq!(allowed(&app, "unknown.test").await, StatusCode::NOT_FOUND);
        assert_eq!(allowed(&app, "not%20a%20domain").await, StatusCode::NOT_FOUND);

        let a = app.tenant("a").await.unwrap();
        assert_eq!(events(&a).await, ["kernel.domain_added", "kernel.domain_verified"]);

        let list = app.get("/api/v1/kernel/domains").token("admin-a").send().await.unwrap();
        assert_eq!(list.status(), StatusCode::OK);
        assert_eq!(list.json()["data"].as_array().unwrap().len(), 1);

        // Thu hồi: không còn được phép, không còn trong danh sách; thu hồi lại không đổi gì.
        let gone = app
            .delete(&format!("/api/v1/kernel/domains/{id}"))
            .token("admin-a")
            .send()
            .await
            .unwrap();
        assert_eq!(gone.status(), StatusCode::NO_CONTENT);
        assert_eq!(allowed(&app, "shop-a.test").await, StatusCode::NOT_FOUND);
        let list = app.get("/api/v1/kernel/domains").token("admin-a").send().await.unwrap();
        assert_eq!(list.json()["data"], json!([]));
        let again = app
            .delete(&format!("/api/v1/kernel/domains/{id}"))
            .token("admin-a")
            .send()
            .await
            .unwrap();
        assert_eq!(again.status(), StatusCode::NO_CONTENT);
        let (status, _) = verify(&app, "admin-a", &id).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(
            events(&a).await,
            ["kernel.domain_added", "kernel.domain_verified", "kernel.domain_revoked"]
        );

        // Không có permission: 403.
        let (status, body) = add(&app, "reader-a", "other-a.test").await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("permission_denied"))
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ten_mien_sai_hay_cua_khach_khac() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        for bad in [
            "203.0.113.7",
            "[2001:db8::1]",
            "localhost",
            "shop.localhost",
            "api.platform.test",
            "API.Platform.Test:443",
            "not a domain",
            "https://shop.test/x",
            "example",
            "",
        ] {
            let (status, body) = add(&app, "admin-a", bad).await;
            assert_eq!(
                (status, body["code"].as_str()),
                (StatusCode::UNPROCESSABLE_ENTITY, Some("tenant_domain_invalid")),
                "{bad}: {body}"
            );
        }
        let (status, body) = add(&app, "admin-a", "shared.test").await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let id = body["data"]["id"].as_str().unwrap().to_owned();
        // Đang chờ ở khách `a`: khách `b` không lấy được.
        let (status, body) = add(&app, "admin-b", "SHARED.test").await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::CONFLICT, Some("tenant_domain_taken")),
            "{body}"
        );

        // Khách `b` không thấy, không xác minh, không xoá được tên miền của `a`.
        let list = app.get("/api/v1/kernel/domains").token("admin-b").send().await.unwrap();
        assert_eq!(list.json()["data"], json!([]));
        let (status, body) = verify(&app, "admin-b", &id).await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("tenant_domain_not_found"))
        );
        let gone = app
            .delete(&format!("/api/v1/kernel/domains/{id}"))
            .token("admin-b")
            .send()
            .await
            .unwrap();
        assert_eq!(gone.status(), StatusCode::NOT_FOUND);
        let list = app.get("/api/v1/kernel/domains").token("admin-a").send().await.unwrap();
        assert_eq!(list.json()["data"].as_array().unwrap().len(), 1);

        // `a` bỏ tên miền: `b` nhận lại được, với token mới.
        let gone = app
            .delete(&format!("/api/v1/kernel/domains/{id}"))
            .token("admin-a")
            .send()
            .await
            .unwrap();
        assert_eq!(gone.status(), StatusCode::NO_CONTENT);
        let (status, body) = add(&app, "admin-b", "shared.test").await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        assert_eq!(body["data"]["status"], "pending");
        let (status, _) = verify(&app, "admin-a", &id).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "the old owner lost it");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn request_theo_ten_mien_da_xac_minh() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test)
            .tenant("a")
            .tenant("b")
            .tenant("locked")
            .domain("a", "shop-a.test")
            .domain("locked", "shop-locked.test")
            .user_in("a", "user-a", 1, &[])
            .user_in("b", "user-b", 2, &[])
            .module(Probe)
            .build()
            .await
            .unwrap();
        // Không đăng nhập: tên miền chọn khách.
        let answer = app.get("/api/v1/probe").host("shop-a.test").send().await.unwrap();
        assert_eq!(answer.status(), StatusCode::OK, "{}", answer.json());
        assert_eq!(answer.json()["data"]["tenant"], "a");
        let answer = app.get("/api/v1/probe").host("Shop-A.test:443").send().await.unwrap();
        assert_eq!(answer.json()["data"]["tenant"], "a");
        // Đăng nhập ở đúng khách: được.
        let answer =
            app.get("/api/v1/probe").host("shop-a.test").token("user-a").send().await.unwrap();
        assert_eq!(answer.json()["data"]["tenant"], "a");
        // Token của khách `b` gửi tới tên miền của `a`: từ chối.
        let answer =
            app.get("/api/v1/probe").host("shop-a.test").token("user-b").send().await.unwrap();
        assert_eq!(
            (answer.status(), answer.json()["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("tenant_mismatch"))
        );
        // Tên miền lạ: không xác định được khách.
        let answer = app.get("/api/v1/probe").host("unknown.test").send().await.unwrap();
        assert_eq!(answer.json()["code"], "tenant_required");

        // Khách bị tạm khoá: không cấp chứng chỉ cho tên miền của nó.
        assert_eq!(allowed(&app, "shop-locked.test").await, StatusCode::OK);
        let admin = app.api::<autoxide_kernel::tenancy::TenantAdmin>().unwrap();
        let locked = app.tenant("locked").await.unwrap();
        let changes = autoxide_kernel::tenancy::TenantChanges {
            name: None,
            status: Some("suspended".to_owned()),
        };
        let actor = autoxide_kernel::event::Actor::system();
        admin.update(actor, locked.id().as_uuid(), changes, None).await.unwrap();
        assert_eq!(allowed(&app, "shop-locked.test").await, StatusCode::NOT_FOUND);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn job_xac_minh_va_het_han() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let (_, ready) = add(&app, "admin-a", "ready.test").await;
        let (_, stale) = add(&app, "admin-a", "stale.test").await;
        let ready_record = &ready["data"]["record"];
        app.dns().set_txt(
            ready_record["name"].as_str().unwrap(),
            &[ready_record["value"].as_str().unwrap()],
        );
        // `stale.test` đã chờ quá 7 ngày (dù có bản ghi đúng cũng hết hạn trước).
        let stale_id = stale["data"]["id"].as_str().unwrap();
        // `ExprTrait` chỉ ở đây: `is_null` của nó trùng tên với `Value::is_null`.
        use sea_orm::sea_query::ExprTrait as _;
        let old = chrono::Utc::now() - chrono::Duration::days(8);
        let backdate = Query::update()
            .table(Alias::new("kernel_tenant_domains"))
            .value(Alias::new("updated_at"), old)
            .and_where(Expr::col(Alias::new("domain")).eq("stale.test"))
            .to_owned();
        assert_eq!(app.database().write().execute(&backdate).await.unwrap().rows_affected(), 1);

        // Lịch của kernel đưa `kernel.verify_domains` vào hàng đợi; worker chạy nó.
        let worker = app.modules().worker(1).unwrap();
        assert!(worker.run_once().await.unwrap() >= 1);
        let list =
            app.get("/api/v1/kernel/domains?order=asc").token("admin-a").send().await.unwrap();
        let rows = list.json()["data"].clone();
        let status_of = |domain: &str| {
            rows.as_array()
                .unwrap()
                .iter()
                .find(|row| row["domain"] == domain)
                .map(|row| (row["status"].clone(), row["check_error"].clone()))
                .unwrap()
        };
        assert_eq!(status_of("ready.test").0, "verified");
        let (status, error) = status_of("stale.test");
        assert_eq!(status, "failed");
        assert!(error.as_str().unwrap().contains("7 days"));
        assert_eq!(allowed(&app, "ready.test").await, StatusCode::OK);

        // `failed` xác minh lại được: chưa có bản ghi thì về `pending`, thêm 7 ngày.
        let (status, body) = verify(&app, "admin-a", stale_id).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["status"], "pending");
        let record = &stale["data"]["record"];
        app.dns().set_txt(record["name"].as_str().unwrap(), &[record["value"].as_str().unwrap()]);
        let service = app.api::<TenantDomains>().unwrap();
        assert_eq!(service.check_pending(app.database()).await.unwrap(), 1);
        assert_eq!(allowed(&app, "stale.test").await, StatusCode::OK);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tool_mcp_dung_chung_service() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let client = app.mcp("admin-a").await.unwrap();
        let added = client
            .call_tool(tool("kernel.add_domain", json!({"domain": "mcp-a.test"})))
            .await
            .unwrap();
        let added = added.structured_content.unwrap()["data"].clone();
        let id = added["id"].as_str().unwrap().to_owned();
        assert_eq!(added["status"], "pending");
        let record = &added["record"];
        app.dns().set_txt(record["name"].as_str().unwrap(), &[record["value"].as_str().unwrap()]);
        let verified =
            client.call_tool(tool("kernel.verify_domain", json!({"id": id}))).await.unwrap();
        assert_eq!(verified.structured_content.unwrap()["data"]["status"], "verified");
        let list = client.call_tool(tool("kernel.list_domains", json!({}))).await.unwrap();
        assert_eq!(list.structured_content.unwrap()["data"][0]["domain"], "mcp-a.test");
        // Tool của khách `b` không thấy tên miền của `a`.
        let other = app.mcp("admin-b").await.unwrap();
        let denied =
            other.call_tool(tool("kernel.verify_domain", json!({"id": id}))).await.unwrap();
        assert_eq!(denied.is_error, Some(true));
        client.cancel().await.unwrap();
        other.cancel().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn che_do_mot_khach_khong_nhan_ten_mien() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test).user("admin", 1, &[MANAGE]).build().await.unwrap();
        let (status, body) = add(&app, "admin", "shop.test").await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::CONFLICT, Some("tenant_single_mode")),
            "{body}"
        );
        assert_eq!(allowed(&app, "shop.test").await, StatusCode::NOT_FOUND);
    })
    .await
    .unwrap();
}
