//! CORS theo tên miền của khách (D-20261125-tenant-cors): origin `https://<tên miền verified>` được preflight
//! và request thật; tên miền pending / bị thu hồi / hết verified thì không; origin của khách A không gọi được
//! API cho khách B (token, header `Autoxide-Tenant`, `Host`); các biến thể origin gần đúng bị từ chối; danh
//! sách chung và `*` giữ hành vi cũ; MCP dùng cùng một nguồn sự thật. Chạy trên cả SQLite và Postgres.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_kernel::error::Error;
use autoxide_kernel::module::Module;
use autoxide_kernel::response::Data;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_testkit::{TestApp, TestDatabase, TestResponse};
use axum::http::{Method, StatusCode, header};
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Query};
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

async fn app(test: &TestDatabase, settings: &[(&str, &str)]) -> TestApp {
    let mut builder = TestApp::builder(test);
    for (name, value) in settings {
        builder = builder.setting(name, value);
    }
    builder
        .tenant("a")
        .tenant("b")
        .domain("a", "shop-a.test")
        .domain("b", "shop-b.test")
        .user_in("a", "user-a", 1, &[MANAGE])
        .user_in("b", "user-b", 2, &[])
        .module(Probe)
        .build()
        .await
        .unwrap()
}

/// Preflight của trình duyệt từ `origin` cho một `POST`.
async fn preflight(app: &TestApp, origin: &str) -> TestResponse {
    app.request(Method::OPTIONS, "/api/v1/probe")
        .header("origin", origin)
        .header("access-control-request-method", "POST")
        .header("access-control-request-headers", "authorization, content-type")
        .unchecked()
        .send()
        .await
        .unwrap()
}

/// Mọi giá trị `Vary` (lớp CORS và lớp khách mỗi lớp thêm một dòng), viết thường.
fn vary(response: &TestResponse) -> String {
    let values = response.headers().get_all(header::VARY).iter();
    values.map(|value| value.to_str().unwrap().to_ascii_lowercase()).collect::<Vec<_>>().join(", ")
}

fn allow_origin(response: &TestResponse) -> &str {
    response.header(header::ACCESS_CONTROL_ALLOW_ORIGIN)
}

#[tokio::test]
async fn ten_mien_da_xac_minh_duoc_cors_va_gan_voi_dung_khach() {
    TestDatabase::run(async |test| {
        let app = app(test, &[]).await;

        // Preflight: origin là tên miền verified của khách → cho phép, đúng origin đó (không `*`).
        let response = preflight(&app, "https://shop-a.test").await;
        assert_eq!(allow_origin(&response), "https://shop-a.test");
        assert!(vary(&response).contains("origin"), "{:?}", response.headers());
        assert!(response.header(header::ACCESS_CONTROL_ALLOW_CREDENTIALS).is_empty());
        assert!(
            response.header(header::ACCESS_CONTROL_ALLOW_HEADERS).contains("authorization"),
            "{:?}",
            response.headers()
        );

        // Request thật đúng khách (token, header, Host): 200 kèm header CORS.
        for request in [
            app.get("/api/v1/probe").token("user-a"),
            app.get("/api/v1/probe").header("autoxide-tenant", "a"),
            app.get("/api/v1/probe").host("shop-a.test"),
        ] {
            let answer = request.header("origin", "https://shop-a.test").send().await.unwrap();
            assert_eq!(answer.status(), StatusCode::OK, "{}", answer.json());
            assert_eq!(answer.json()["data"]["tenant"], "a");
            assert_eq!(allow_origin(&answer), "https://shop-a.test");
            assert!(vary(&answer).contains("origin"));
        }

        // Origin của khách `a`, request cho khách `b` (token, header, Host): từ chối trước handler. Header CORS
        // vẫn có để trang đọc được lỗi rõ ràng.
        for request in [
            app.get("/api/v1/probe").token("user-b"),
            app.get("/api/v1/probe").header("autoxide-tenant", "b"),
            app.get("/api/v1/probe").host("shop-b.test"),
        ] {
            let answer = request.header("origin", "https://shop-a.test").send().await.unwrap();
            assert_eq!(
                (answer.status(), answer.json()["code"].as_str()),
                (StatusCode::FORBIDDEN, Some("tenant_origin_mismatch")),
                "{}",
                answer.json()
            );
        }
        // Ghi cũng bị chặn trước khi chạy: khách `b` không có tên miền mới.
        let write = app
            .post("/api/v1/kernel/domains")
            .token("user-b")
            .header("origin", "https://shop-a.test")
            .json(json!({"domain": "sneaky-b.test"}))
            .send()
            .await
            .unwrap();
        assert_eq!(write.json()["code"], "tenant_origin_mismatch");

        // Không có Origin (máy gọi máy): không đổi gì.
        let answer = app.get("/api/v1/probe").token("user-b").send().await.unwrap();
        assert_eq!(answer.json()["data"]["tenant"], "b");
        assert!(allow_origin(&answer).is_empty());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn origin_gan_dung_khong_duoc_chap_nhan() {
    TestDatabase::run(async |test| {
        let app = app(test, &[]).await;
        for origin in [
            "null",
            "http://shop-a.test",
            "https://SHOP-A.test",
            "https://shop-a.test.",
            "https://shop-a.test:443",
            "https://shop-a.test:8443",
            "https://shop-a.test/",
            "https://evil.shop-a.test",
            "https://shop-a.test.evil.example",
            "https://xshop-a.test",
            "https://shop-a.tes",
            "https://unknown.test",
            "https://api.platform.test",
        ] {
            let response = preflight(&app, origin).await;
            assert!(allow_origin(&response).is_empty(), "{origin}: {:?}", response.headers());
            let answer = app
                .get("/api/v1/probe")
                .token("user-a")
                .header("origin", origin)
                .send()
                .await
                .unwrap();
            // Không được phép thì không có header CORS (trình duyệt chặn đọc); máy gọi máy vẫn chạy.
            assert!(allow_origin(&answer).is_empty(), "{origin}");
            assert_eq!(answer.status(), StatusCode::OK, "{origin}");
        }
    })
    .await
    .unwrap();
}

/// Đặt trạng thái của tên miền `domain` thẳng trong bảng — như máy chủ khác vừa đổi: cache của máy này chưa
/// biết.
async fn set_status(app: &TestApp, domain: &str, status: &str) {
    let update = Query::update()
        .table(Alias::new("kernel_tenant_domains"))
        .value(Alias::new("status"), status)
        .and_where(Expr::col(Alias::new("domain")).eq(domain))
        .to_owned();
    app.database().write().execute(&update).await.unwrap();
}

#[tokio::test]
async fn pending_failed_va_thu_hoi_khong_con_cors() {
    TestDatabase::run(async |test| {
        let app = app(test, &[]).await;

        // `pending`: chưa chứng minh sở hữu → không cấp CORS.
        let added = app
            .post("/api/v1/kernel/domains")
            .token("user-a")
            .json(json!({"domain": "pending-a.test"}))
            .send()
            .await
            .unwrap();
        assert_eq!(added.status(), StatusCode::CREATED, "{}", added.json());
        assert!(allow_origin(&preflight(&app, "https://pending-a.test").await).is_empty());

        // Máy khác đổi sang `failed`: máy này còn cho tới khi cache hết hạn (TTL
        // `AUTOXIDE_TENANCY_CACHE_SECONDS`) hay bị xoá; sau đó mất quyền.
        assert_eq!(
            allow_origin(&preflight(&app, "https://shop-b.test").await),
            "https://shop-b.test"
        );
        set_status(&app, "shop-b.test", "failed").await;
        assert_eq!(
            allow_origin(&preflight(&app, "https://shop-b.test").await),
            "https://shop-b.test"
        );
        app.registry().invalidate_domain("shop-b.test");
        assert!(allow_origin(&preflight(&app, "https://shop-b.test").await).is_empty());

        // Thu hồi qua API trên máy này: mất quyền ngay.
        assert_eq!(
            allow_origin(&preflight(&app, "https://shop-a.test").await),
            "https://shop-a.test"
        );
        let list = app.get("/api/v1/kernel/domains").token("user-a").send().await.unwrap();
        let id = list.json()["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["domain"] == "shop-a.test")
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let gone = app
            .delete(&format!("/api/v1/kernel/domains/{id}"))
            .token("user-a")
            .send()
            .await
            .unwrap();
        assert_eq!(gone.status(), StatusCode::NO_CONTENT);
        assert!(allow_origin(&preflight(&app, "https://shop-a.test").await).is_empty());
        let answer = app
            .get("/api/v1/probe")
            .token("user-a")
            .header("origin", "https://shop-a.test")
            .send()
            .await
            .unwrap();
        assert!(allow_origin(&answer).is_empty());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn danh_sach_chung_giu_hanh_vi_cu() {
    TestDatabase::run(async |test| {
        // Origin trong danh sách chung: không gắn với khách nào (giao diện quản trị của nền tảng).
        let app = app(test, &[("AUTOXIDE_HTTP_CORS_ORIGINS", "https://admin.platform.test")]).await;
        assert_eq!(
            allow_origin(&preflight(&app, "https://admin.platform.test").await),
            "https://admin.platform.test"
        );
        for token in ["user-a", "user-b"] {
            let answer = app
                .get("/api/v1/probe")
                .token(token)
                .header("origin", "https://admin.platform.test")
                .send()
                .await
                .unwrap();
            assert_eq!(answer.status(), StatusCode::OK);
            assert_eq!(allow_origin(&answer), "https://admin.platform.test");
        }
        // Tên miền của khách vẫn chạy song song với danh sách chung.
        assert_eq!(
            allow_origin(&preflight(&app, "https://shop-a.test").await),
            "https://shop-a.test"
        );
        assert!(allow_origin(&preflight(&app, "https://evil.test").await).is_empty());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dau_sao_giu_hanh_vi_cu() {
    TestDatabase::run(async |test| {
        // `*`: mọi origin, header là `*` như trước; không gắn khách.
        let app = app(test, &[("AUTOXIDE_HTTP_CORS_ORIGINS", "*")]).await;
        assert_eq!(allow_origin(&preflight(&app, "https://anything.test").await), "*");
        let answer = app
            .get("/api/v1/probe")
            .token("user-b")
            .header("origin", "https://shop-a.test")
            .send()
            .await
            .unwrap();
        assert_eq!((answer.status(), allow_origin(&answer)), (StatusCode::OK, "*"));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn mot_khach_khong_tra_ten_mien() {
    TestDatabase::run(async |test| {
        // Chế độ một khách: chỉ danh sách chung, không truy vấn tên miền.
        let app = TestApp::builder(test)
            .setting("AUTOXIDE_HTTP_CORS_ORIGINS", "https://app.example.com")
            .module(Probe)
            .build()
            .await
            .unwrap();
        assert_eq!(
            allow_origin(&preflight(&app, "https://app.example.com").await),
            "https://app.example.com"
        );
        assert!(allow_origin(&preflight(&app, "https://shop-a.test").await).is_empty());
    })
    .await
    .unwrap();
}

/// Gửi `tools/list` tới `/mcp` từ `origin` bằng token `token`.
async fn mcp(app: &TestApp, origin: &str, token: &str) -> TestResponse {
    app.post("/mcp")
        .token(token)
        .header("origin", origin)
        .header("accept", "application/json, text/event-stream")
        .json(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))
        .unchecked()
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn mcp_dung_cung_nguon_origin() {
    TestDatabase::run(async |test| {
        let app = app(test, &[]).await;
        // Origin của khách, token của khách đó: được.
        let answer = mcp(&app, "https://shop-a.test", "user-a").await;
        assert_eq!(answer.status(), StatusCode::OK, "{}", answer.text());
        assert!(answer.json()["result"]["tools"].is_array(), "{}", answer.text());
        // Origin lạ: chống DNS rebinding như trước.
        let answer = mcp(&app, "https://evil.test", "user-a").await;
        assert_eq!(answer.status(), StatusCode::FORBIDDEN, "{}", answer.text());
        // Origin của khách `a`, token của khách `b`: từ chối.
        let answer = mcp(&app, "https://shop-a.test", "user-b").await;
        assert_eq!(
            (answer.status(), answer.json()["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("tenant_origin_mismatch")),
            "{}",
            answer.text()
        );
    })
    .await
    .unwrap();
}
