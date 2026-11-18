//! `TestApp` dựng app như binary: cấu hình sai thì báo tên biến, trait đưa qua `provide` tới
//! được module cần nó, bộ xác thực giả nhận đúng token đã khai — trên cả hai database.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::config::{ConfigReader, Invalid, Setting};
use autoxide_kernel::error::Error;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::response::Data;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::extract::State;
use axum::http::StatusCode;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

/// Trait một module "khác" công bố; ở đây test đưa vào bằng `provide`.
trait Greeting: Send + Sync {
    fn text(&self) -> String;
}

struct Hello;

impl Greeting for Hello {
    fn text(&self) -> String {
        "hello".to_owned()
    }
}

static GREETING_NAME: Setting =
    Setting::new("AUTOXIDE_GREET_NAME", "Who to greet.").with_default("world");

#[derive(Default)]
struct Greet {
    name: String,
    greeting: Option<Arc<dyn Greeting>>,
}

impl Module for Greet {
    fn name(&self) -> &'static str {
        "greet"
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.name = reader.get_with(&GREETING_NAME, |raw| {
            if raw.chars().all(char::is_alphabetic) {
                Ok(raw.to_owned())
            } else {
                Err("must contain letters only".to_owned())
            }
        })?;
        Ok(())
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        self.greeting = Some(context.require::<dyn Greeting>()?);
        Ok(())
    }

    fn routes(&self) -> OpenApiRouter {
        let text = format!(
            "{} {}",
            self.greeting.as_ref().map_or_else(String::new, |g| g.text()),
            self.name
        );
        OpenApiRouter::new().routes(routes!(greet)).with_state(Arc::new(text))
    }
}

/// Greet the caller.
#[utoipa::path(
    get,
    path = "/",
    responses(
        (status = 200, description = "The greeting.", body = Data<String>),
        (status = 401, description = "Not signed in.", body = autoxide_kernel::response::Problem),
    )
)]
async fn greet(State(text): State<Arc<String>>, _principal: Principal) -> Data<String> {
    Data { data: text.as_str().to_owned() }
}

#[tokio::test]
async fn dung_app_goi_rest_voi_token_da_khai() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test)
            .setting("AUTOXIDE_GREET_NAME", "ada")
            .provide::<dyn Greeting>(Arc::new(Hello))
            .user("ada", 1, &[])
            .module(Greet::default())
            .build()
            .await
            .unwrap();

        let answer = app.get("/api/v1/greet").token("ada").send().await.unwrap();
        assert_eq!(answer.status(), StatusCode::OK, "{}", answer.text());
        assert_eq!(answer.json()["data"], "hello ada");

        let answer = app.get("/api/v1/greet").token("stranger").send().await.unwrap();
        assert_eq!(answer.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(answer.json()["code"], "auth.invalid_token");

        // Lớp HTTP của kernel có mặt như ở binary.
        let health = app.get("/health/ready").send().await.unwrap();
        assert_eq!(health.status(), StatusCode::OK);
        assert!(!answer.header("x-request-id").is_empty());
    })
    .await
    .unwrap();
}

/// Module chỉ có một route trả bên gọi: người dùng nào, qua API key nào.
struct WhoAmI;

impl Module for WhoAmI {
    fn name(&self) -> &'static str {
        "whoami"
    }

    fn routes(&self) -> OpenApiRouter {
        OpenApiRouter::new().routes(routes!(who_am_i)).routes(routes!(count_bytes))
    }
}

/// Count the bytes of the request body.
#[utoipa::path(
    post,
    path = "/bytes",
    responses((status = 200, description = "Number of bytes received.", body = Data<usize>))
)]
async fn count_bytes(body: String) -> Data<usize> {
    Data { data: body.len() }
}

/// Who is calling: user id and API key id (`null` without one).
#[utoipa::path(
    get,
    path = "/",
    responses(
        (status = 200, description = "User id, then API key id.", body = Data<Vec<Option<String>>>),
        (status = 401, description = "Not signed in.", body = autoxide_kernel::response::Problem),
    )
)]
async fn who_am_i(principal: Principal) -> Data<Vec<Option<String>>> {
    let key = principal.api_key_id().map(|id| id.to_string());
    Data { data: vec![Some(principal.user_id().to_string()), key] }
}

/// `principal` khai bên gọi mà `user` không tả được, vd gọi bằng API key.
#[tokio::test]
async fn principal_dung_san_vd_api_key() {
    TestDatabase::run(async |test| {
        let agent = Principal::new(Uuid::from_u128(2), []).via_api_key(Uuid::from_u128(3));
        let app = TestApp::builder(test)
            .user("ada", 1, &[])
            .principal("agent", agent)
            .module(WhoAmI)
            .build()
            .await
            .unwrap();

        let ada = app.get("/api/v1/whoami").token("ada").send().await.unwrap();
        assert_eq!(ada.json()["data"], serde_json::json!([Uuid::from_u128(1), null]));
        let agent = app.get("/api/v1/whoami").token("agent").send().await.unwrap();
        assert_eq!(agent.status(), StatusCode::OK, "{}", agent.text());
        assert_eq!(
            agent.json()["data"],
            serde_json::json!([Uuid::from_u128(2), Uuid::from_u128(3)])
        );
    })
    .await
    .unwrap();
}

/// `body` gửi thân bất kỳ, kể cả gửi theo từng đoạn (không có `Content-Length`).
#[tokio::test]
async fn gui_than_theo_tung_doan() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test).module(WhoAmI).build().await.unwrap();
        let parts = ["hello ", "world"].map(|part| Ok::<_, std::io::Error>(part.to_owned()));
        let stream = axum::body::Body::from_stream(futures_util::stream::iter(parts));
        let answer = app.post("/api/v1/whoami/bytes").body(stream).send().await.unwrap();
        assert_eq!(answer.status(), StatusCode::OK, "{}", answer.text());
        assert_eq!(answer.json()["data"], 11);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cau_hinh_sai_hay_thieu_trait_thi_bao_ro() {
    TestDatabase::run(async |test| {
        let blank = TestApp::builder(test)
            .setting("AUTOXIDE_GREET_NAME", "ada1")
            .provide::<dyn Greeting>(Arc::new(Hello))
            .module(Greet::default())
            .build()
            .await;
        let error = blank.err().unwrap().to_string();
        assert!(error.contains("AUTOXIDE_GREET_NAME"), "{error}");

        let missing = TestApp::builder(test).module(Greet::default()).build().await;
        let error = missing.err().unwrap().to_string();
        assert!(error.starts_with("test app: start"), "{error}");
    })
    .await
    .unwrap();
}

/// Module có route khai sai: tài liệu hứa chuỗi, code trả số.
struct Liar;

impl Module for Liar {
    fn name(&self) -> &'static str {
        "liar"
    }

    fn routes(&self) -> OpenApiRouter {
        OpenApiRouter::new().routes(routes!(lie))
    }
}

/// Promise a string.
#[utoipa::path(
    get,
    path = "/",
    responses((status = 200, description = "A string, supposedly.", body = Data<String>))
)]
async fn lie() -> Data<i32> {
    Data { data: 7 }
}

/// Mọi request qua `send()` được kiểm theo `docs/openapi.json` của chính app (đặc tả 31 B4).
#[tokio::test]
async fn response_lech_hop_dong_thi_send_bao_loi() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test).module(Liar).build().await.unwrap();
        let error = app.get("/api/v1/liar").send().await.unwrap_err().to_string();
        assert!(
            error.contains("response does not match docs/openapi.json: GET /api/v1/liar 200: $.data: expected string"),
            "{error}"
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn unchecked_bo_qua_kiem() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test).module(Liar).build().await.unwrap();
        let answer = app.get("/api/v1/liar").unchecked().send().await.unwrap();
        assert_eq!(answer.json()["data"], 7);
    })
    .await
    .unwrap();
}
