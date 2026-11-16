//! Header bảo mật cho mọi response (bước 40 P1, D-320): có trên 200, 404 của router, preflight
//! CORS, `/health/live`, `/mcp`; handler đặt CSP riêng thì giữ; trang tài liệu có CSP riêng; HSTS
//! chỉ khi cấu hình; tắt được bằng cấu hình; `Referrer-Policy` sai bị từ chối lúc đọc cấu hình.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
// Clippy chỉ tự miễn cho hàm `#[test]`, không miễn cho hàm phụ, nên khai ở đây.
// Trong `src/` thì KHÔNG được khai dòng này (`cargo xtask check` chặn).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_kernel::config::{ConfigReader, ConfigSource, HttpConfig};
use autoxide_kernel::http::{API_CSP, DOCS_CSP};
use autoxide_kernel::module::Module;
use autoxide_testkit::{TestApp, TestDatabase, TestResponse};
use axum::http::{Method, header};
use axum::response::IntoResponse;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

/// Return a file with its own, stricter policy.
#[utoipa::path(get, path = "/own", extensions(("x-public" = json!(true))), responses((status = 200, description = "A page with its own policy.", body = String, content_type = "text/plain")))]
async fn own_policy() -> axum::response::Response {
    ([(header::CONTENT_SECURITY_POLICY, "default-src 'none'")], "own").into_response()
}

/// Module thử: một route tự đặt CSP.
struct Pages;

impl Module for Pages {
    fn name(&self) -> &'static str {
        "pages"
    }

    fn routes(&self) -> OpenApiRouter {
        OpenApiRouter::new().routes(routes!(own_policy))
    }
}

async fn app(test: &TestDatabase, settings: &[(&str, &str)]) -> TestApp {
    let mut builder = TestApp::builder(test).module(Pages).user("ada", 1, &[]);
    for (name, value) in settings {
        builder = builder.setting(name, value);
    }
    builder.build().await.unwrap()
}

fn assert_secured(response: &TestResponse, what: &str) {
    assert_eq!(response.header(header::X_CONTENT_TYPE_OPTIONS), "nosniff", "{what}");
    assert_eq!(response.header(header::REFERRER_POLICY), "no-referrer", "{what}");
    assert_eq!(response.header(header::X_FRAME_OPTIONS), "DENY", "{what}");
    assert_eq!(response.header(header::CONTENT_SECURITY_POLICY), API_CSP, "{what}");
    assert!(!response.headers().contains_key(header::STRICT_TRANSPORT_SECURITY), "{what}");
}

#[tokio::test]
async fn moi_response_co_header_bao_mat() {
    TestDatabase::run(async |test| {
        let app = app(test, &[("AUTOXIDE_HTTP_CORS_ORIGINS", "https://app.example.com")]).await;

        let ok = app.get("/api/v1/kernel/client-config").send().await.unwrap();
        assert_eq!(ok.status().as_u16(), 200);
        assert_secured(&ok, "200 JSON");
        let missing = app.get("/api/v1/nothing-here").send().await.unwrap();
        assert_eq!(missing.status().as_u16(), 404);
        assert_secured(&missing, "404 của router");
        let preflight = app
            .request(Method::OPTIONS, "/api/v1/kernel/client-config")
            .header("origin", "https://app.example.com")
            .header("access-control-request-method", "GET")
            .send()
            .await
            .unwrap();
        assert!(preflight.status().is_success(), "{}", preflight.status());
        assert_secured(&preflight, "preflight CORS");
        let live = app.get("/health/live").send().await.unwrap();
        assert_eq!(live.status().as_u16(), 200);
        assert_secured(&live, "/health/live");
        let mcp = app.post("/mcp").send().await.unwrap();
        assert_eq!(mcp.status().as_u16(), 401);
        assert_secured(&mcp, "/mcp không đăng nhập");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn handler_dat_csp_rieng_thi_giu() {
    TestDatabase::run(async |test| {
        let app = app(test, &[]).await;
        let own = app.get("/api/v1/pages/own").send().await.unwrap();
        assert_eq!(own.header(header::CONTENT_SECURITY_POLICY), "default-src 'none'");
        // Header khác vẫn được thêm.
        assert_eq!(own.header(header::X_CONTENT_TYPE_OPTIONS), "nosniff");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn trang_tai_lieu_co_csp_rieng() {
    TestDatabase::run(async |test| {
        let app = app(test, &[("AUTOXIDE_HTTP_API_DOCS", "true")]).await;
        let docs = app.get("/api/docs").send().await.unwrap();
        assert_eq!(docs.status().as_u16(), 200);
        assert_eq!(docs.header(header::CONTENT_SECURITY_POLICY), DOCS_CSP);
        // Trang nạp đúng script mà CSP cho phép.
        let page = String::from_utf8(docs.body().to_vec()).unwrap();
        assert!(page.contains("https://cdn.jsdelivr.net/"), "{page}");
        // Tài liệu JSON vẫn theo CSP của API.
        let document = app.get("/api/v1/openapi.json").send().await.unwrap();
        assert_eq!(document.header(header::CONTENT_SECURITY_POLICY), API_CSP);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hsts_chi_khi_cau_hinh() {
    TestDatabase::run(async |test| {
        let settings = [
            ("AUTOXIDE_HTTP_HSTS_SECONDS", "31536000"),
            ("AUTOXIDE_HTTP_HSTS_INCLUDE_SUBDOMAINS", "true"),
            ("AUTOXIDE_HTTP_REFERRER_POLICY", "strict-origin-when-cross-origin"),
        ];
        let app = app(test, &settings).await;
        let live = app.get("/health/live").send().await.unwrap();
        assert_eq!(
            live.header(header::STRICT_TRANSPORT_SECURITY),
            "max-age=31536000; includeSubDomains"
        );
        assert_eq!(live.header(header::REFERRER_POLICY), "strict-origin-when-cross-origin");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tat_header_bang_cau_hinh() {
    TestDatabase::run(async |test| {
        let app = app(test, &[("AUTOXIDE_HTTP_SECURITY_HEADERS", "false")]).await;
        let live = app.get("/health/live").send().await.unwrap();
        for name in [
            header::X_CONTENT_TYPE_OPTIONS,
            header::REFERRER_POLICY,
            header::X_FRAME_OPTIONS,
            header::CONTENT_SECURITY_POLICY,
        ] {
            assert!(!live.headers().contains_key(&name), "{name}");
        }
    })
    .await
    .unwrap();
}

#[test]
fn cau_hinh_referrer_policy_sai_bi_tu_choi() {
    let read = |pairs: &[(&str, &str)]| {
        let source = ConfigSource::from_pairs(pairs.iter().copied());
        let mut reader = ConfigReader::new(&source);
        let config = HttpConfig::read(&mut reader);
        reader.finish(config).map(|(config, _)| config).map_err(|error| error.to_string())
    };
    let error = read(&[("AUTOXIDE_HTTP_REFERRER_POLICY", "always")]).unwrap_err();
    assert!(error.contains("AUTOXIDE_HTTP_REFERRER_POLICY"), "{error}");
    let error = read(&[("AUTOXIDE_HTTP_HSTS_SECONDS", "99999999999")]).unwrap_err();
    assert!(error.contains("AUTOXIDE_HTTP_HSTS_SECONDS"), "{error}");
    let config = read(&[]).unwrap();
    assert!(config.security_headers.enabled);
    assert_eq!(config.security_headers.referrer_policy, "no-referrer");
    assert_eq!(config.security_headers.hsts_seconds, 0);
}
