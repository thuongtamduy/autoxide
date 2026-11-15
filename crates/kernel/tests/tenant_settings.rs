//! Cấu hình theo khách (bước 33 A7): đặt / đọc / bỏ qua REST, secret lưu mã hoá và không bao giờ
//! trả ra (API, event), khách khác không thấy, cấu hình không ai khai bị từ chối. Chạy trên cả
//! SQLite và Postgres.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use autoxide_kernel::access::{Authenticator, Principal};
use autoxide_kernel::config::{ConfigReader, ConfigSource, DatabaseConfig, HttpConfig};
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::http;
use autoxide_kernel::module::{Module, Modules};
use autoxide_kernel::tenancy::{
    MAIN_PLACEMENT, TenantContext, TenantHint, TenantId, TenantSetting, TenantSettings,
    TenantStatus,
};
use autoxide_testkit::TestDatabase;
use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use sea_orm::{ConnectionTrait, Statement};
use serde_json::{Value, json};
use tower::ServiceExt as _;
use uuid::Uuid;

const API_KEY: TenantSetting =
    TenantSetting::new("demo.api_key", "Key of the demo partner.").secret();
const GREETING: TenantSetting = TenantSetting::new("demo.greeting", "Greeting shown on the site.");

struct Demo;

impl Module for Demo {
    fn name(&self) -> &'static str {
        "demo"
    }

    fn tenant_settings(&self) -> Vec<TenantSetting> {
        vec![API_KEY, GREETING]
    }
}

struct FakeAuthenticator;

#[async_trait::async_trait]
impl Authenticator for FakeAuthenticator {
    async fn authenticate(
        &self,
        authorization: Option<&str>,
        _: &TenantHint,
    ) -> Result<Principal, Error> {
        match authorization {
            Some("Bearer admin") => {
                Ok(Principal::new(Uuid::nil(), ["kernel.settings.manage".to_owned()]))
            }
            Some("Bearer nobody") => Ok(Principal::new(Uuid::nil(), [])),
            _ => Err(Error::unauthenticated("auth.missing_token", "Sign in.")),
        }
    }
}

async fn start(test: &TestDatabase) -> (Database, Modules, Router) {
    let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
    let mut reader = ConfigReader::new(&source);
    let database = DatabaseConfig::read(&mut reader);
    let http_config = HttpConfig::read(&mut reader);
    let mut modules = Modules::new(vec![Box::new(Demo)]).unwrap();
    let configured = modules.configure(&mut reader);
    let (database, http_config) = reader
        .finish(database.and_then(|database| {
            http_config.and_then(|config| configured.map(|()| (database, config)))
        }))
        .unwrap()
        .0;
    let db = Database::connect(&database).await.unwrap();
    modules.migrator().unwrap().up(&db).await.unwrap();
    modules.start(&db).unwrap();
    let extras = http::Extras::default()
        .authenticator(Some(Arc::new(FakeAuthenticator)))
        .tools(modules.tools().unwrap())
        .translations(modules.translations().clone())
        .tenancy(modules.tenancy().unwrap());
    let router = http::build(
        &http_config,
        http::health_routes(db.clone(), http::Draining::default()),
        modules.routes(),
        extras,
    )
    .router;
    (db, modules, router)
}

async fn call(
    app: &Router,
    method: Method,
    path: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {token}"));
    if body.is_some() {
        request = request.header(header::CONTENT_TYPE, "application/json");
    }
    let body = body.map_or_else(Body::empty, |body| Body::from(body.to_string()));
    let response = app.clone().oneshot(request.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn texts(database: &Database, sql: &str) -> Vec<String> {
    let backend = database.read().get_database_backend();
    let rows = database.read().query_all_raw(Statement::from_string(backend, sql)).await.unwrap();
    rows.iter().map(|row| row.try_get_by_index::<String>(0).unwrap()).collect()
}

#[tokio::test]
async fn setting_secret_khong_bao_gio_tra_ra() {
    TestDatabase::run(async |test| {
        let (database, modules, app) = start(test).await;
        let secret = "not-a-real-partner-value";
        let path = "/api/v1/kernel/settings/demo.api_key";
        let (status, body) =
            call(&app, Method::PUT, path, "admin", Some(json!({"value": secret}))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            (body["data"]["is_set"].clone(), body["data"]["value"].clone()),
            (json!(true), Value::Null)
        );
        let greeting = "/api/v1/kernel/settings/demo.greeting";
        call(&app, Method::PUT, greeting, "admin", Some(json!({"value": "Xin chào"}))).await;

        let (status, list) =
            call(&app, Method::GET, "/api/v1/kernel/settings", "admin", None).await;
        assert_eq!(status, StatusCode::OK, "{list}");
        assert!(!list.to_string().contains(secret), "{list}");
        let names: Vec<&str> =
            list["data"].as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["demo.api_key", "demo.greeting", "kernel.public_url"]);
        assert_eq!(list["data"][1]["value"], "Xin chào");
        assert_eq!(list["data"][2]["is_set"], false);

        // Database chỉ giữ bản mã hoá; event chỉ mang tên.
        let stored =
            texts(&database, "SELECT value FROM kernel_tenant_settings ORDER BY name").await;
        assert!(stored[0].starts_with("sealed:v1:") && !stored[0].contains(secret), "{stored:?}");
        let events = texts(&database, "SELECT payload FROM kernel_events").await;
        assert_eq!(events.len(), 2);
        assert!(
            events.iter().all(|payload| !payload.contains(secret) && !payload.contains("Xin chào"))
        );

        // Module đọc được giá trị thật; khách khác không thấy gì.
        let settings = modules.api::<dyn TenantSettings>().unwrap();
        let ours = TenantContext::default_on(database.clone());
        let value = settings.get(&ours, &API_KEY).await.unwrap().unwrap();
        assert_eq!(value.as_str(), secret);
        assert!(!format!("{value:?}").contains(secret), "Debug hides secrets");
        let theirs = TenantContext::new(
            TenantId::from_uuid(Uuid::now_v7()),
            "other",
            "Other",
            TenantStatus::Active,
            MAIN_PLACEMENT,
            database.clone(),
        );
        assert!(settings.get(&theirs, &API_KEY).await.unwrap().is_none());

        // Bỏ: về cấu hình chung.
        let (status, _) = call(&app, Method::DELETE, path, "admin", None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(settings.get(&ours, &API_KEY).await.unwrap().is_none());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cau_hinh_la_hay_thieu_quyen_bi_tu_choi() {
    TestDatabase::run(async |test| {
        let (_, _, app) = start(test).await;
        let path = "/api/v1/kernel/settings/demo.missing";
        let (status, body) =
            call(&app, Method::PUT, path, "admin", Some(json!({"value": "x"}))).await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("tenant_setting_unknown"))
        );
        let (status, body) =
            call(&app, Method::GET, "/api/v1/kernel/settings", "nobody", None).await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("permission_denied"))
        );
        let path = "/api/v1/kernel/settings/demo.greeting";
        let (status, _) =
            call(&app, Method::PUT, path, "admin", Some(json!({"value": "  "}))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}

#[test]
fn khai_cau_hinh_sai_bi_tu_choi() {
    struct Wrong;
    impl Module for Wrong {
        fn name(&self) -> &'static str {
            "wrong"
        }
        fn tenant_settings(&self) -> Vec<TenantSetting> {
            vec![
                TenantSetting::new("other.key", "Not mine."),
                TenantSetting::new("wrong.Key", "Upper case."),
                TenantSetting::new("wrong.empty", " "),
                TenantSetting::new("wrong.twice", "Once."),
                TenantSetting::new("wrong.twice", "Twice."),
            ]
        }
    }
    let error = Modules::new(vec![Box::new(Wrong)]).err().unwrap().to_string();
    for part in [
        "`other.key`",
        "`wrong.Key`",
        "`wrong.empty` has no description",
        "`wrong.twice` is declared twice",
    ] {
        assert!(error.contains(part), "{part}: {error}");
    }
}
