//! Tên miền riêng của khách (bước 33 C1, D-190): thêm → bản ghi TXT phải đặt; xác minh bằng DNS giả;
//! `GET /domains/allowed` cho Caddy; tên miền sai / của khách khác; thu hồi; request theo `Host`;
//! job `kernel.verify_domains`; khách khác không đụng được tên miền của mình; gắn tên miền với một
//! nút của cây tổ chức (`PATCH /domains/{id}`, đối chiếu thiết kế đợt 2). Chạy trên cả SQLite và
//! Postgres.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::response::Data;
use autoxide_kernel::tenancy::{OrgId, OrgNodes, TenantContext, TenantDomains};
use autoxide_testkit::{TestApp, TestDatabase, tool};
use axum::http::StatusCode;
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Expr, Query};
use serde_json::{Value, json};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

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
            status: Some("suspended".to_owned()),
            ..Default::default()
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
        let old = Timestamp::now() - SignedDuration::from_hours(24 * 8);
        let backdate = Query::update()
            .table(Alias::new("kernel_tenant_domains"))
            .value(Alias::new("updated_at"), old)
            .and_where(Expr::col(Alias::new("domain")).eq("stale.test"))
            .to_owned();
        assert_eq!(app.database().write().execute(&backdate).await.unwrap().rows_affected(), 1);

        // Lịch của kernel đưa `kernel.verify_domains` vào hàng đợi; worker chạy nó.
        let worker = app.modules().worker(2).unwrap();
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

/// Lùi `last_checked_at` của `domain` về 25 giờ trước: lần kiểm lại hằng ngày tới hạn.
async fn backdate_check(app: &TestApp, domain: &str) {
    use sea_orm::sea_query::ExprTrait as _;
    let old = Timestamp::now() - SignedDuration::from_hours(25);
    let update = Query::update()
        .table(Alias::new("kernel_tenant_domains"))
        .value(Alias::new("last_checked_at"), old)
        .and_where(Expr::col(Alias::new("domain")).eq(domain))
        .to_owned();
    assert_eq!(app.database().write().execute(&update).await.unwrap().rows_affected(), 1);
}

/// Tên miền `domain` của khách `a`, qua REST.
async fn domain_of(app: &TestApp, domain: &str) -> Value {
    let list = app.get("/api/v1/kernel/domains").token("admin-a").send().await.unwrap();
    let rows = list.json()["data"].as_array().unwrap().clone();
    rows.into_iter().find(|row| row["domain"] == domain).unwrap()
}

/// D-291 mục 8: tên miền `verified` được kiểm lại mỗi ngày; 3 lần liên tiếp không thấy bản ghi →
/// `failed` (event, không còn được phép); thấy lại thì đếm từ 0; DNS không trả lời không tính.
#[tokio::test]
async fn kiem_lai_hang_ngay_ten_mien_da_xac_minh() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let service = app.api::<TenantDomains>().unwrap();
        let (_, added) = add(&app, "admin-a", "daily.test").await;
        let id = added["data"]["id"].as_str().unwrap().to_owned();
        let record = added["data"]["record"].clone();
        let (name, value) = (record["name"].as_str().unwrap(), record["value"].as_str().unwrap());
        app.dns().set_txt(name, &[value]);
        let (_, verified) = verify(&app, "admin-a", &id).await;
        assert_eq!(
            (verified["data"]["status"].as_str(), verified["data"]["failed_checks"].as_u64()),
            (Some("verified"), Some(0))
        );
        // Vừa kiểm: chưa tới hạn.
        assert_eq!(service.recheck_verified(app.database()).await.unwrap(), 0);

        // Bản ghi còn: vẫn `verified`, không trượt.
        backdate_check(&app, "daily.test").await;
        assert_eq!(service.recheck_verified(app.database()).await.unwrap(), 1);
        assert_eq!(domain_of(&app, "daily.test").await["failed_checks"], 0);

        // Mất bản ghi: trượt lần 1, vẫn được phép.
        app.dns().remove(name);
        backdate_check(&app, "daily.test").await;
        service.recheck_verified(app.database()).await.unwrap();
        let row = domain_of(&app, "daily.test").await;
        assert_eq!(
            (row["status"].as_str(), row["failed_checks"].as_u64()),
            (Some("verified"), Some(1))
        );
        assert!(row["check_error"].as_str().unwrap().contains("No TXT record"), "{row}");
        assert_eq!(allowed(&app, "daily.test").await, StatusCode::OK);

        // DNS không trả lời: không tính.
        app.dns().fail(true);
        backdate_check(&app, "daily.test").await;
        service.recheck_verified(app.database()).await.unwrap();
        let row = domain_of(&app, "daily.test").await;
        assert_eq!(row["failed_checks"], 1);
        assert!(row["check_error"].as_str().unwrap().contains("DNS lookup failed"), "{row}");
        app.dns().fail(false);

        // Trượt lần 2, rồi bản ghi trở lại: đếm lại từ 0.
        backdate_check(&app, "daily.test").await;
        service.recheck_verified(app.database()).await.unwrap();
        assert_eq!(domain_of(&app, "daily.test").await["failed_checks"], 2);
        app.dns().set_txt(name, &[value]);
        backdate_check(&app, "daily.test").await;
        service.recheck_verified(app.database()).await.unwrap();
        let row = domain_of(&app, "daily.test").await;
        assert_eq!((row["failed_checks"].as_u64(), &row["check_error"]), (Some(0), &Value::Null));

        // Ba lần liên tiếp (lần cuối do job của kernel chạy): `failed`, không còn vào khách.
        app.dns().remove(name);
        for _ in 0..2 {
            backdate_check(&app, "daily.test").await;
            service.recheck_verified(app.database()).await.unwrap();
        }
        assert_eq!(allowed(&app, "daily.test").await, StatusCode::OK);
        backdate_check(&app, "daily.test").await;
        let worker = app.modules().worker(2).unwrap();
        assert!(worker.run_once().await.unwrap() >= 1);
        let row = domain_of(&app, "daily.test").await;
        assert_eq!(
            (row["status"].as_str(), row["failed_checks"].as_u64()),
            (Some("failed"), Some(3))
        );
        assert_eq!(allowed(&app, "daily.test").await, StatusCode::NOT_FOUND);
        let a = app.tenant("a").await.unwrap();
        assert_eq!(
            events(&a).await,
            ["kernel.domain_added", "kernel.domain_verified", "kernel.domain_failed"]
        );

        // Đặt lại bản ghi rồi xác minh: `verified`, đếm từ 0.
        app.dns().set_txt(name, &[value]);
        let (_, again) = verify(&app, "admin-a", &id).await;
        assert_eq!(
            (again["data"]["status"].as_str(), again["data"]["failed_checks"].as_u64()),
            (Some("verified"), Some(0))
        );
        assert_eq!(allowed(&app, "daily.test").await, StatusCode::OK);
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

/// Cây tổ chức giả: mỗi khách (theo slug) có đúng một nút.
struct Orgs(Vec<(&'static str, Uuid)>);

#[async_trait::async_trait]
impl OrgNodes for Orgs {
    async fn has_node(&self, tenant: &TenantContext, org: OrgId) -> Result<bool, Error> {
        Ok(self.0.iter().any(|(slug, id)| *slug == tenant.slug() && *id == org.as_uuid()))
    }
}

/// Module thử công bố cây tổ chức giả, như module `organizations` của bước 34 sẽ làm.
struct OrgsModule(Vec<(&'static str, Uuid)>);

impl Module for OrgsModule {
    fn name(&self) -> &'static str {
        "orgs"
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        context.provide::<dyn OrgNodes>(Arc::new(Orgs(self.0.clone())))
    }
}

/// Payload các event `kernel.domain_updated` của khách `tenant`, theo thứ tự ghi.
async fn updates(tenant: &TenantContext) -> Vec<Value> {
    use sea_orm::sea_query::ExprTrait as _;
    let select = Query::select()
        .column(Alias::new("payload"))
        .from(Alias::new("kernel_events"))
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
        .and_where(Expr::col(Alias::new("name")).eq("kernel.domain_updated"))
        .order_by(Alias::new("id"), sea_orm::sea_query::Order::Asc)
        .to_owned();
    let rows = tenant.database().read().query_all(&select).await.unwrap();
    rows.iter()
        .map(|row| serde_json::from_str(&row.try_get::<String>("", "payload").unwrap()).unwrap())
        .collect()
}

async fn attach(app: &TestApp, token: &str, id: &str, body: Value) -> (StatusCode, Value) {
    let answer = app
        .patch(&format!("/api/v1/kernel/domains/{id}"))
        .token(token)
        .json(body)
        .send()
        .await
        .unwrap();
    (answer.status(), answer.json())
}

#[tokio::test]
async fn gan_ten_mien_voi_nut_cua_khach() {
    TestDatabase::run(async |test| {
        let (store_a, store_b) = (Uuid::now_v7(), Uuid::now_v7());
        let app = TestApp::builder(test)
            .tenant("a")
            .tenant("b")
            .user_in("a", "admin-a", 1, &[MANAGE])
            .user_in("b", "admin-b", 2, &[MANAGE])
            .user_in("a", "reader-a", 3, &[])
            .module(OrgsModule(vec![("a", store_a), ("b", store_b)]))
            .build()
            .await
            .unwrap();
        let (_, added) = add(&app, "admin-a", "store-a.test").await;
        let id = added["data"]["id"].as_str().unwrap().to_owned();
        assert!(added["data"]["org_id"].is_null(), "{added}");

        // Gắn với nút của khách: trả `org_id`, không đổi trạng thái hay token.
        let (status, body) = attach(&app, "admin-a", &id, json!({"org_id": store_a})).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["data"]["org_id"], json!(store_a));
        assert_eq!(body["data"]["status"], "pending");
        assert_eq!(body["data"]["record"], added["data"]["record"]);
        assert_eq!(body["data"]["updated_at"], added["data"]["updated_at"]);
        let a = app.tenant("a").await.unwrap();
        assert_eq!(events(&a).await, ["kernel.domain_added", "kernel.domain_updated"]);
        // Gửi lại đúng giá trị: không đổi gì, không thêm event.
        let (status, _) = attach(&app, "admin-a", &id, json!({"org_id": store_a})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(events(&a).await.len(), 2);
        // Danh sách cũng mang `org_id`.
        let list = app.get("/api/v1/kernel/domains").token("admin-a").send().await.unwrap();
        assert_eq!(list.json()["data"][0]["org_id"], json!(store_a));

        // Nút của khách khác, nút không có: 422 ở ô `org_id`, không đổi gì.
        for other in [store_b, Uuid::now_v7()] {
            let (status, body) = attach(&app, "admin-a", &id, json!({"org_id": other})).await;
            assert_eq!(
                (status, body["code"].as_str(), body["errors"][0]["field"].as_str()),
                (StatusCode::UNPROCESSABLE_ENTITY, Some("validation_failed"), Some("org_id")),
                "{body}"
            );
            assert_eq!(body["errors"][0]["code"], "not_found", "{body}");
        }
        // Thiếu `org_id` (phải gửi `null` rõ ràng), sai kiểu, thừa trường: 422.
        for bad in [json!({}), json!({"org_id": "store"}), json!({"org_id": null, "status": "x"})] {
            let (status, body) = attach(&app, "admin-a", &id, bad.clone()).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}: {body}");
        }
        // Khách khác không đụng được; thiếu permission: 403; không có: 404.
        let (status, body) = attach(&app, "admin-b", &id, json!({"org_id": store_b})).await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("tenant_domain_not_found")),
            "{body}"
        );
        let (status, body) = attach(&app, "reader-a", &id, json!({"org_id": null})).await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("permission_denied"))
        );
        let missing = Uuid::now_v7().to_string();
        let (status, _) = attach(&app, "admin-a", &missing, json!({"org_id": null})).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let list = app.get("/api/v1/kernel/domains").token("admin-a").send().await.unwrap();
        assert_eq!(list.json()["data"][0]["org_id"], json!(store_a));

        // Xác minh xong vẫn giữ nút; gỡ bằng `null`.
        let record = &added["data"]["record"];
        app.dns().set_txt(record["name"].as_str().unwrap(), &[record["value"].as_str().unwrap()]);
        let (_, verified) = verify(&app, "admin-a", &id).await;
        assert_eq!(
            (&verified["data"]["status"], &verified["data"]["org_id"]),
            (&json!("verified"), &json!(store_a))
        );
        let (status, body) = attach(&app, "admin-a", &id, json!({"org_id": null})).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body["data"]["org_id"].is_null());
        assert_eq!(body["data"]["status"], "verified");
        assert_eq!(
            events(&a).await,
            [
                "kernel.domain_added",
                "kernel.domain_updated",
                "kernel.domain_verified",
                "kernel.domain_updated"
            ]
        );
        // Event mang nút mới và nút cũ.
        let payloads = updates(&a).await;
        assert_eq!(
            (&payloads[0]["org_id"], &payloads[0]["previous_org_id"]),
            (&json!(store_a), &Value::Null)
        );
        assert_eq!(
            (&payloads[1]["org_id"], &payloads[1]["previous_org_id"]),
            (&Value::Null, &json!(store_a))
        );

        // Tool MCP dùng chung service.
        let client = app.mcp("admin-a").await.unwrap();
        let updated = client
            .call_tool(tool("kernel.update_domain", json!({"id": id, "org_id": store_a})))
            .await
            .unwrap();
        assert_eq!(updated.structured_content.unwrap()["data"]["org_id"], json!(store_a));
        let denied = client
            .call_tool(tool("kernel.update_domain", json!({"id": id, "org_id": store_b})))
            .await
            .unwrap();
        assert_eq!(denied.is_error, Some(true));
        client.cancel().await.unwrap();

        // Đã thu hồi: 409, kể cả khi gửi `null`. Thêm lại tên miền thì bắt đầu không gắn nút nào.
        let gone = app
            .delete(&format!("/api/v1/kernel/domains/{id}"))
            .token("admin-a")
            .send()
            .await
            .unwrap();
        assert_eq!(gone.status(), StatusCode::NO_CONTENT);
        for body in [json!({"org_id": null}), json!({"org_id": store_a})] {
            let (status, answer) = attach(&app, "admin-a", &id, body).await;
            assert_eq!(
                (status, answer["code"].as_str()),
                (StatusCode::CONFLICT, Some("tenant_domain_revoked")),
                "{answer}"
            );
        }
        let (_, again) = add(&app, "admin-a", "store-a.test").await;
        assert!(again["data"]["org_id"].is_null(), "{again}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn chua_co_cay_to_chuc_thi_chi_nhan_null() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let (_, added) = add(&app, "admin-a", "plain-a.test").await;
        let id = added["data"]["id"].as_str().unwrap().to_owned();
        let (status, body) = attach(&app, "admin-a", &id, json!({"org_id": Uuid::now_v7()})).await;
        assert_eq!(
            (status, body["errors"][0]["field"].as_str(), body["errors"][0]["code"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("org_id"), Some("unsupported")),
            "{body}"
        );
        assert!(body["errors"][0]["detail"].as_str().unwrap().contains("send `null`"), "{body}");
        let (status, body) = attach(&app, "admin-a", &id, json!({"org_id": null})).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body["data"]["org_id"].is_null());
        let a = app.tenant("a").await.unwrap();
        assert_eq!(events(&a).await, ["kernel.domain_added"], "nothing changed, no event");
    })
    .await
    .unwrap();
}
