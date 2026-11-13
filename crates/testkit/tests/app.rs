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
        (status = 200, description = "The greeting.", body = String),
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
