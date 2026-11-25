//! CORS theo tên miền của khách (D-20261125-tenant-cors): origin `https://<tên miền verified>` được preflight
//! và request thật; tên miền pending / bị thu hồi / hết verified thì không; origin của khách A không gọi được
//! API cho khách B (token, header `Autoxide-Tenant`, `Host`); các biến thể origin gần đúng bị từ chối; danh
//! sách chung và `*` giữ hành vi cũ; MCP dùng cùng một nguồn sự thật. Chạy trên cả SQLite và Postgres.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_kernel::error::Error;
use autoxide_kernel::module::Module;
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::{
    DatabaseRegistry, OptionalTenant, TenantContext, link_token, resolve_link_token,
};
use autoxide_testkit::{TestApp, TestDatabase, TestResponse};
use axum::Extension;
use axum::http::{Method, StatusCode, header};
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Query};
use serde::Deserialize;
use serde_json::{Value, json};
use utoipa::IntoParams;
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

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
struct RedeemQuery {
    /// The link token.
    token: String,
}

/// Tell which tenant a link token belongs to.
///
/// Like `/auth/refresh`: the tenant comes from the token, the request may not name one.
#[utoipa::path(
    get,
    path = "/redeem",
    extensions(("x-public" = json!(true))),
    params(RedeemQuery),
    responses(
        (status = 200, description = "The tenant of the token.", body = Data<Value>),
        (status = 403, description = "The page is on another tenant's domain.", body = Problem),
        (status = 404, description = "The token is wrong.", body = Problem),
    )
)]
async fn redeem(
    Extension(registry): Extension<DatabaseRegistry>,
    OptionalTenant(tenant): OptionalTenant,
    axum::extract::Query(query): axum::extract::Query<RedeemQuery>,
) -> Result<Data<Value>, Error> {
    match resolve_link_token(&registry, tenant.as_ref(), &query.token).await? {
        Some((tenant, _)) => Ok(Data { data: json!({"tenant": tenant.slug()}) }),
        None => Err(Error::not_found("probe.token_invalid", "The token is wrong.")),
    }
}

impl Module for Probe {
    fn name(&self) -> &'static str {
        "probe"
    }

    fn routes(&self) -> OpenApiRouter {
        OpenApiRouter::new().routes(routes!(whoami)).routes(routes!(redeem))
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
        .user("ops", 9, &["kernel.tenants.operate"])
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
            assert_eq!(allow_origin(&answer), "https://shop-a.test");
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

/// Nhân sự nền tảng (khách mặc định, có `kernel.tenants.operate`) mở trang trên tên miền của khách `a`: chỉ làm
/// được cho `a` (vào bằng `Autoxide-Tenant: a`); làm cho chính nền tảng hay vào khách `b` từ trang đó bị từ chối —
/// JavaScript trên tên miền của khách không được điều khiển quyền cấp nền tảng.
#[tokio::test]
async fn tai_khoan_nen_tang_tu_origin_cua_khach() {
    TestDatabase::run(async |test| {
        let app = app(test, &[]).await;
        let call = |tenant: Option<&'static str>| {
            let request =
                app.get("/api/v1/probe").token("ops").header("origin", "https://shop-a.test");
            match tenant {
                Some(tenant) => request.header("autoxide-tenant", tenant),
                None => request,
            }
        };
        let answer = call(Some("a")).send().await.unwrap();
        assert_eq!(answer.status(), StatusCode::OK, "{}", answer.json());
        assert_eq!(answer.json()["data"]["tenant"], "a");
        for tenant in [None, Some("b")] {
            let answer = call(tenant).send().await.unwrap();
            assert_eq!(answer.json()["code"], "tenant_origin_mismatch", "{tenant:?}");
        }
        // Từ giao diện nền tảng (không có Origin, hay origin trong danh sách chung) thì như cũ.
        let answer = app.get("/api/v1/probe").token("ops").send().await.unwrap();
        assert_eq!(answer.status(), StatusCode::OK, "{}", answer.json());
    })
    .await
    .unwrap();
}

/// Origin lạ chỉ là một lần tra bản đồ trong bộ nhớ: dội nhiều origin ngẫu nhiên không làm origin hợp lệ bị
/// từ chối.
#[tokio::test]
async fn doi_origin_la_khong_chan_origin_hop_le() {
    TestDatabase::run(async |test| {
        let app = app(test, &[]).await;
        for index in 0..300 {
            let origin = format!("https://random-{index}.test");
            assert!(allow_origin(&preflight(&app, &origin).await).is_empty(), "{origin}");
        }
        assert_eq!(
            allow_origin(&preflight(&app, "https://shop-a.test").await),
            "https://shop-a.test"
        );
        assert_eq!(
            allow_origin(&preflight(&app, "https://shop-b.test").await),
            "https://shop-b.test"
        );
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

/// Endpoint lấy khách từ token (như `/auth/refresh`, link trong mail): trang trên tên miền của khách `a` không dùng
/// được token của khách `b` — khách của request suy từ origin, token của khách khác như token sai.
#[tokio::test]
async fn token_cua_khach_khac_tu_origin_cua_khach() {
    TestDatabase::run(async |test| {
        let app = app(test, &[]).await;
        let a = app.tenant("a").await.unwrap();
        let b = app.tenant("b").await.unwrap();
        let redeem = |token: String, origin: Option<&'static str>| {
            let request = app.get(&format!("/api/v1/probe/redeem?token={token}"));
            match origin {
                Some(origin) => request.header("origin", origin),
                None => request,
            }
        };
        // Không có Origin (máy gọi máy, trang ở tên miền chính): token tự nêu khách như trước.
        let answer = redeem(link_token(&b, "raw"), None).send().await.unwrap();
        assert_eq!(answer.json()["data"]["tenant"], "b", "{}", answer.json());
        // Từ trang của `a`: token của `a` được, token của `b` như token sai.
        let answer =
            redeem(link_token(&a, "raw"), Some("https://shop-a.test")).send().await.unwrap();
        assert_eq!(answer.json()["data"]["tenant"], "a", "{}", answer.json());
        let answer =
            redeem(link_token(&b, "raw"), Some("https://shop-a.test")).send().await.unwrap();
        assert_eq!(
            (answer.status(), answer.json()["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("probe.token_invalid")),
            "{}",
            answer.json()
        );
        // Trực tiếp: request đã có khách `a`, token ghi khách `b` → không có.
        let token = link_token(&b, "raw");
        let found = resolve_link_token(app.registry(), Some(&a), &token).await;
        assert!(found.unwrap().is_none());
    })
    .await
    .unwrap();
}

/// Database điều khiển không đọc được: origin của khách bị từ chối (fail-closed), nhanh, không treo request;
/// đọc lại được thì cho lại.
#[tokio::test]
async fn database_loi_thi_tu_choi_origin_cua_khach() {
    TestDatabase::run(async |test| {
        let app = app(test, &[]).await;
        let rename = |from: &str, to: &str| format!("ALTER TABLE {from} RENAME TO {to}");
        let write = app.database().write();
        write
            .execute_unprepared(&rename("kernel_tenant_domains", "kernel_tenant_domains_off"))
            .await
            .unwrap();
        app.registry().invalidate_domain("shop-a.test");
        let started = std::time::Instant::now();
        for _ in 0..20 {
            assert!(allow_origin(&preflight(&app, "https://shop-a.test").await).is_empty());
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "{:?}", started.elapsed());
        write
            .execute_unprepared(&rename("kernel_tenant_domains_off", "kernel_tenant_domains"))
            .await
            .unwrap();
        // Lỗi được nhớ tối đa 1 giây rồi đọc lại.
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        assert_eq!(
            allow_origin(&preflight(&app, "https://shop-a.test").await),
            "https://shop-a.test"
        );
    })
    .await
    .unwrap();
}
