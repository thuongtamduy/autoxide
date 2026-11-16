//! Cấu hình theo khách (bước 33 A7): đặt / đọc / bỏ qua REST, secret lưu mã hoá và không bao giờ
//! trả ra (API, event), khách khác không thấy, cấu hình không ai khai bị từ chối; nhóm `connection`
//! (lọc `?group=`, quy ước tên) và tên cũ của cấu hình đã đổi tên (đối chiếu thiết kế đợt 2). Chạy
//! trên cả SQLite và Postgres.

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
    CONNECTION_GROUP, MAIN_PLACEMENT, TenantContext, TenantHint, TenantId, TenantSetting,
    TenantSettings, TenantStatus,
};
use autoxide_testkit::{TestApp, TestDatabase, tool};
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

// Module `demo` ở bản sau: hai cấu hình trên thành kết nối đối tác, đổi tên theo quy ước.
const PARTNER_KEY: TenantSetting =
    TenantSetting::new("demo.connection.partner.key", "Key of the demo partner.")
        .secret()
        .group(CONNECTION_GROUP)
        .formerly("demo.api_key");
const PARTNER_ACCOUNT: TenantSetting =
    TenantSetting::new("demo.connection.partner.account", "Account at the demo partner.")
        .group(CONNECTION_GROUP)
        .formerly("demo.greeting");
const COLOR: TenantSetting = TenantSetting::new("demo.color", "Colour of the site.");

struct Renamed;

impl Module for Renamed {
    fn name(&self) -> &'static str {
        "demo"
    }

    fn tenant_settings(&self) -> Vec<TenantSetting> {
        vec![PARTNER_KEY, PARTNER_ACCOUNT, COLOR]
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
    start_with(test, Box::new(Demo)).await
}

async fn start_with(test: &TestDatabase, module: Box<dyn Module>) -> (Database, Modules, Router) {
    let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
    let mut reader = ConfigReader::new(&source);
    let database = DatabaseConfig::read(&mut reader);
    let http_config = HttpConfig::read(&mut reader);
    let mut modules = Modules::new(vec![module]).unwrap();
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
                TenantSetting::new("wrong.a..b", "Empty part."),
                TenantSetting::new("wrong.shade", "Bad group.").group("Bad Group"),
                // Nhóm `connection` và tên `<module>.connection.<loại>.<trường>` đi cùng nhau.
                TenantSetting::new("wrong.connection.sms", "Too short.").group(CONNECTION_GROUP),
                TenantSetting::new("wrong.plain", "Not named so.").group(CONNECTION_GROUP),
                TenantSetting::new("wrong.connection.sms.key", "No group."),
                // Tên cũ: phải của module, không trùng tên nào khác.
                TenantSetting::new("wrong.next", "Old name of another.").formerly("other.old"),
                TenantSetting::new("wrong.newer", "Old name in use.").formerly("wrong.shade"),
                // Hợp lệ: nhiều đoạn, đúng quy ước.
                TenantSetting::new("wrong.connection.pay.vnpay.key", "Fine.")
                    .secret()
                    .group(CONNECTION_GROUP),
                TenantSetting::new("wrong.a.b", "Fine."),
            ]
        }
    }
    let error = Modules::new(vec![Box::new(Wrong)]).err().unwrap().to_string();
    for part in [
        "`other.key`",
        "`wrong.Key`",
        "`wrong.empty` has no description",
        "`wrong.twice` is declared twice",
        "`wrong.a..b` of module `wrong` must be named",
        "`wrong.shade` is in group `Bad Group`",
        "`wrong.connection.sms` is in group `connection`, so it must be named",
        "`wrong.plain` is in group `connection`, so it must be named",
        "`wrong.connection.sms.key` is named like a partner connection",
        "has the former name `other.old`",
        "`wrong.shade` is declared twice",
    ] {
        assert!(error.contains(part), "{part}: {error}");
    }
    for fine in ["`wrong.connection.pay.vnpay.key`", "`wrong.a.b`"] {
        assert!(!error.contains(fine), "{fine}: {error}");
    }
}

async fn names(database: &Database) -> Vec<String> {
    texts(database, "SELECT name FROM kernel_tenant_settings ORDER BY name").await
}

/// Đối chiếu thiết kế đợt 2: cấu hình kết nối đối tác đổi tên theo quy ước
/// `<module>.connection.<loại>.<trường>`; giá trị khách đã đặt dưới tên cũ (kể cả secret, vốn mã
/// hoá kèm tên cũ) vẫn dùng được, API vẫn nhận tên cũ, lần đặt kế tiếp ghi dưới tên mới.
#[tokio::test]
async fn doi_ten_cau_hinh_gia_tri_cu_van_dung_duoc() {
    TestDatabase::run(async |test| {
        // Bản cũ: khách đặt hai cấu hình dưới tên cũ.
        let (database, _, old) = start(test).await;
        let secret = "not-a-real-partner-value";
        for (name, value) in [("demo.api_key", secret), ("demo.greeting", "Xin chào")] {
            let path = format!("/api/v1/kernel/settings/{name}");
            let (status, body) =
                call(&old, Method::PUT, &path, "admin", Some(json!({"value": value}))).await;
            assert_eq!(status, StatusCode::OK, "{body}");
        }
        assert_eq!(names(&database).await, ["demo.api_key", "demo.greeting"]);

        // Bản mới trên cùng database: tên mới, khai tên cũ.
        let (database, modules, app) = start_with(test, Box::new(Renamed)).await;
        let settings = modules.api::<dyn TenantSettings>().unwrap();
        let ours = TenantContext::default_on(database.clone());
        let value = settings.get(&ours, &PARTNER_KEY).await.unwrap().unwrap();
        assert_eq!(value.as_str(), secret, "a secret sealed under the former name still opens");
        let account = settings.get(&ours, &PARTNER_ACCOUNT).await.unwrap().unwrap();
        assert_eq!(account.as_str(), "Xin chào");
        let (status, list) =
            call(&app, Method::GET, "/api/v1/kernel/settings?group=connection", "admin", None)
                .await;
        assert_eq!(status, StatusCode::OK, "{list}");
        assert!(!list.to_string().contains(secret), "{list}");
        let listed: Vec<(&str, bool, &Value, &Value)> = list["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| (s["name"].as_str().unwrap(), s["is_set"] == true, &s["value"], &s["group"]))
            .collect();
        let connection = json!("connection");
        assert_eq!(
            listed,
            [
                ("demo.connection.partner.account", true, &json!("Xin chào"), &connection),
                ("demo.connection.partner.key", true, &Value::Null, &connection),
            ]
        );
        // Không lọc: mọi cấu hình, cấu hình không nhóm có `group: null`; nhóm lạ: rỗng.
        let (_, all) = call(&app, Method::GET, "/api/v1/kernel/settings", "admin", None).await;
        let all: Vec<(&str, &Value)> = all["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| (s["name"].as_str().unwrap(), &s["group"]))
            .collect();
        assert_eq!(
            all,
            [
                ("demo.color", &Value::Null),
                ("demo.connection.partner.account", &connection),
                ("demo.connection.partner.key", &connection),
                ("kernel.public_url", &Value::Null),
            ]
        );
        let (status, none) =
            call(&app, Method::GET, "/api/v1/kernel/settings?group=nothing", "admin", None).await;
        assert_eq!((status, &none["data"]), (StatusCode::OK, &json!([])));

        // Đặt qua TÊN CŨ: vẫn nhận, ghi dưới tên mới, dòng tên cũ bị bỏ; event mang tên mới.
        let path = "/api/v1/kernel/settings/demo.api_key";
        let (status, body) =
            call(&app, Method::PUT, path, "admin", Some(json!({"value": "second-value"}))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            (&body["data"]["name"], &body["data"]["group"], &body["data"]["value"]),
            (&json!("demo.connection.partner.key"), &connection, &Value::Null)
        );
        assert_eq!(names(&database).await, ["demo.connection.partner.key", "demo.greeting"]);
        let value = settings.get(&ours, &PARTNER_KEY).await.unwrap().unwrap();
        assert_eq!(value.as_str(), "second-value");
        let events = texts(&database, "SELECT payload FROM kernel_events ORDER BY id").await;
        assert!(events.last().unwrap().contains("demo.connection.partner.key"), "{events:?}");

        // Bỏ qua tên mới: bỏ luôn giá trị còn nằm dưới tên cũ.
        let path = "/api/v1/kernel/settings/demo.connection.partner.account";
        let (status, _) = call(&app, Method::DELETE, path, "admin", None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(settings.get(&ours, &PARTNER_ACCOUNT).await.unwrap().is_none());
        assert_eq!(names(&database).await, ["demo.connection.partner.key"]);
        // Bỏ qua tên cũ cũng được.
        let (status, _) =
            call(&app, Method::DELETE, "/api/v1/kernel/settings/demo.api_key", "admin", None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(names(&database).await.is_empty());
        assert!(settings.get(&ours, &PARTNER_KEY).await.unwrap().is_none());

        // Bản giả của testkit theo cùng luật: tên hiện tại trước, không có thì tên cũ.
        let fake = autoxide_testkit::FakeTenantSettings::default();
        fake.set(ours.id(), "demo.api_key", "old-name");
        let value = fake.get(&ours, &PARTNER_KEY).await.unwrap().unwrap();
        assert_eq!(value.as_str(), "old-name");
        fake.set(ours.id(), "demo.connection.partner.key", "new-name");
        let value = fake.get(&ours, &PARTNER_KEY).await.unwrap().unwrap();
        assert_eq!(value.as_str(), "new-name");
    })
    .await
    .unwrap();
}

/// Lọc theo nhóm qua route của nhân sự nền tảng và qua tool MCP: cùng service với REST.
#[tokio::test]
async fn loc_nhom_cho_nhan_su_nen_tang_va_tool_mcp() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test)
            .tenant("shop")
            .user_in("shop", "owner", 1, &["kernel.settings.manage"])
            .user("platform", 2, &["kernel.tenants.read", "kernel.tenants.manage"])
            .module(Renamed)
            .build()
            .await
            .unwrap();
        let path = "/api/v1/kernel/tenants/shop/settings/demo.connection.partner.account";
        let set = app.put(path).token("platform").json(json!({"value": "acct-1"})).send().await;
        let set = set.unwrap();
        assert_eq!(set.status(), StatusCode::OK, "{}", set.json());
        let listed = app
            .get("/api/v1/kernel/tenants/shop/settings?group=connection")
            .token("platform")
            .send()
            .await
            .unwrap();
        assert_eq!(listed.status(), StatusCode::OK, "{}", listed.json());
        let listed = listed.json();
        let rows = listed["data"].as_array().unwrap();
        assert_eq!(rows.len(), 2, "{listed}");
        assert_eq!(
            (&rows[0]["name"], &rows[0]["value"], &rows[0]["group"]),
            (&json!("demo.connection.partner.account"), &json!("acct-1"), &json!("connection"))
        );

        let client = app.mcp("owner").await.unwrap();
        let only = client
            .call_tool(tool("kernel.list_settings", json!({"group": "connection"})))
            .await
            .unwrap();
        let only = only.structured_content.unwrap()["data"].clone();
        let names: Vec<&str> =
            only.as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["demo.connection.partner.account", "demo.connection.partner.key"]);
        assert_eq!(only[0]["value"], "acct-1");
        let all = client.call_tool(tool("kernel.list_settings", json!({}))).await.unwrap();
        assert_eq!(all.structured_content.unwrap()["data"].as_array().unwrap().len(), 4);
        client.cancel().await.unwrap();
    })
    .await
    .unwrap();
}
