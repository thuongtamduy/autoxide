//! Quản lý khách (bước 33 A9): tạo khách gọi bộ gieo của mọi module và cấp phát lại được khi hỏng
//! giữa chừng; chỉ nhân sự của khách nền tảng quản lý được khách; khách bị khoá nhận 403; chế độ
//! một khách không tạo được khách. Chạy trên cả SQLite và Postgres.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use autoxide_kernel::access::{Authenticator, Principal};
use autoxide_kernel::config::{Config, ConfigReader, ConfigSource, HttpConfig};
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::http;
use autoxide_kernel::module::{Module, Modules};
use autoxide_kernel::tenancy::{
    DEFAULT_TENANT, DatabaseRegistry, NewTenant, TenancyConfig, TenancyMode, TenantAdmin,
    TenantChanges, TenantContext, TenantHint, TenantId, TenantSeed,
};
use autoxide_testkit::TestDatabase;
use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt as _;
use uuid::Uuid;

/// Bộ gieo đếm số lần chạy; `fail` bật thì hỏng (như cấp phát chết giữa chừng).
#[derive(Default)]
struct Counting {
    runs: AtomicUsize,
    fail: AtomicBool,
}

#[async_trait::async_trait]
impl TenantSeed for Counting {
    async fn seed(&self, tenant: &TenantContext) -> Result<(), Error> {
        assert!(!tenant.is_platform());
        self.runs.fetch_add(1, Ordering::SeqCst);
        if self.fail.load(Ordering::SeqCst) {
            return Err(Error::internal("the seed failed"));
        }
        Ok(())
    }
}

struct Shop(Arc<Counting>);

impl Module for Shop {
    fn name(&self) -> &'static str {
        "shop"
    }

    fn seed_tenant(&self) -> Option<Arc<dyn TenantSeed>> {
        Some(self.0.clone())
    }
}

/// Token `platform` (khách nền tảng, mọi quyền quản lý khách); `tenant:<id>` (người của khách
/// `<id>`, cùng quyền); `operator:<id>` (nhân sự nền tảng đang vào khách `<id>`).
struct FakeAuthenticator;

const ALL: [&str; 3] = ["kernel.tenants.read", "kernel.tenants.manage", "kernel.placements.read"];

#[async_trait::async_trait]
impl Authenticator for FakeAuthenticator {
    async fn authenticate(
        &self,
        authorization: Option<&str>,
        _: &TenantHint,
    ) -> Result<Principal, Error> {
        let token = authorization.and_then(|value| value.strip_prefix("Bearer "));
        let principal = Principal::new(Uuid::now_v7(), ALL.map(str::to_owned));
        match token.and_then(|token| token.split_once(':')) {
            None if token == Some("platform") => Ok(principal),
            Some(("tenant", id)) => {
                Ok(principal.in_tenant(TenantId::from_uuid(id.parse().unwrap())))
            }
            _ => Err(Error::unauthenticated("auth.missing_token", "Sign in.")),
        }
    }
}

struct Setup {
    modules: Modules,
    admin: Arc<TenantAdmin>,
    registry: DatabaseRegistry,
    app: Router,
    seed: Arc<Counting>,
}

async fn setup(test: &TestDatabase, mode: TenancyMode) -> Setup {
    let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
    let mut reader = ConfigReader::new(&source);
    let config = Config::read(&mut reader);
    let http_config = HttpConfig::read(&mut reader);
    let seed = Arc::new(Counting::default());
    let mut modules = Modules::new(vec![Box::new(Shop(seed.clone()))]).unwrap();
    let configured = modules.configure(&mut reader);
    let (config, http_config) =
        reader
            .finish(config.and_then(|config| {
                http_config.and_then(|http| configured.map(|()| (config, http)))
            }))
            .unwrap()
            .0;
    let database = Database::connect(&config.database).await.unwrap();
    modules.migrator().unwrap().up(&database).await.unwrap();
    let registry =
        DatabaseRegistry::new(database.clone(), config.database, TenancyConfig::with_mode(mode));
    modules.start_in(&registry).unwrap();
    let admin = modules.api::<TenantAdmin>().unwrap();
    let extras = http::Extras::default()
        .authenticator(Some(Arc::new(FakeAuthenticator)))
        .translations(modules.translations().clone())
        .tenancy(registry.clone());
    let app = http::build(
        &http_config,
        http::health_routes(database, http::Draining::default()),
        modules.routes(),
        extras,
    )
    .router;
    Setup { modules, admin, registry, app, seed }
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

fn new_tenant(slug: &str) -> NewTenant {
    serde_json::from_value(json!({"slug": slug, "name": slug.to_uppercase()})).unwrap()
}

#[tokio::test]
async fn tao_khach_goi_seed_cua_moi_module_va_chay_lai_duoc() {
    TestDatabase::run(async |test| {
        let setup = setup(test, TenancyMode::Multi).await;
        // Cấp phát chết giữa chừng: khách nằm lại `provisioning`.
        setup.seed.fail.store(true, Ordering::SeqCst);
        assert!(setup.admin.create(Actor::system(), new_tenant("acme")).await.is_err());
        let stuck = setup.admin.get("acme").await.unwrap();
        assert_eq!(stuck.status, "provisioning");
        // Chạy lại: gieo lại từ đầu, rồi `active`; chạy thêm lần nữa không hỏng gì.
        setup.seed.fail.store(false, Ordering::SeqCst);
        let ready = setup.admin.provision(Actor::system(), "acme").await.unwrap();
        assert_eq!(ready.status, "active");
        setup.admin.provision(Actor::system(), "acme").await.unwrap();
        assert_eq!(setup.seed.runs.load(Ordering::SeqCst), 3);
        assert_eq!(setup.registry.tenant_by_slug("acme").await.unwrap().id().as_uuid(), ready.id);

        // Qua REST: tạo (201 + Location), trùng slug, placement không có.
        let (status, body) = call(
            &setup.app,
            Method::POST,
            "/api/v1/kernel/tenants",
            "platform",
            Some(json!({"slug": "beta", "name": "Beta"})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        assert_eq!(body["data"]["status"], "active");
        assert_eq!(body["data"]["placement"], "main");
        // `Location` là đường dẫn đầy đủ, có tiền tố `/api/v1` (giao diện báo thiếu ngày 16/11/2026).
        let request = Request::builder()
            .method(Method::POST)
            .uri("/api/v1/kernel/tenants")
            .header(header::AUTHORIZATION, "Bearer platform")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({"slug": "delta", "name": "Delta"}).to_string()))
            .unwrap();
        let response = setup.app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let location = response.headers()[header::LOCATION].to_str().unwrap().to_owned();
        assert!(location.starts_with("/api/v1/kernel/tenants/"), "{location}");
        let (status, body) = call(
            &setup.app,
            Method::POST,
            "/api/v1/kernel/tenants",
            "platform",
            Some(json!({"slug": "beta", "name": "Again"})),
        )
        .await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::CONFLICT, Some("tenant_slug_taken"))
        );
        let (status, body) = call(
            &setup.app,
            Method::POST,
            "/api/v1/kernel/tenants",
            "platform",
            Some(json!({"slug": "gamma", "name": "G", "placement": "nowhere"})),
        )
        .await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("tenant_placement_not_found"))
        );
        let (status, body) = call(
            &setup.app,
            Method::POST,
            "/api/v1/kernel/tenants",
            "platform",
            Some(json!({"slug": "-bad-", "name": ""})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        let (status, page) =
            call(&setup.app, Method::GET, "/api/v1/kernel/tenants?status=active", "platform", None)
                .await;
        assert_eq!(status, StatusCode::OK);
        let slugs: Vec<&str> =
            page["data"].as_array().unwrap().iter().map(|t| t["slug"].as_str().unwrap()).collect();
        assert_eq!(slugs, ["delta", "beta", "acme", "default"]);
        drop(setup.modules);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn admin_khach_thuong_khong_quan_ly_duoc_khach_khac() {
    TestDatabase::run(async |test| {
        let setup = setup(test, TenancyMode::Multi).await;
        let acme = setup.admin.create(Actor::system(), new_tenant("acme")).await.unwrap();
        let token = format!("tenant:{}", acme.id);
        // Người của khách thường có mọi permission vẫn không quản lý được khách.
        for (method, path, body) in [
            (Method::GET, "/api/v1/kernel/tenants".to_owned(), None),
            (Method::GET, format!("/api/v1/kernel/tenants/{}", acme.id), None),
            (
                Method::POST,
                "/api/v1/kernel/tenants".to_owned(),
                Some(json!({"slug": "x", "name": "X"})),
            ),
            (Method::GET, "/api/v1/kernel/placements".to_owned(), None),
        ] {
            let (status, body) = call(&setup.app, method, &path, &token, body).await;
            assert_eq!(
                (status, body["code"].as_str()),
                (StatusCode::FORBIDDEN, Some("tenant_platform_only")),
                "{path}"
            );
        }
        // Nhưng xem được khách của chính mình.
        let (status, body) =
            call(&setup.app, Method::GET, "/api/v1/kernel/tenant", &token, None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            (body["data"]["slug"].as_str(), body["data"]["platform"].as_bool()),
            (Some("acme"), Some(false))
        );
        assert_eq!(body["data"]["mode"], "multi");
        // Nhân sự nền tảng đang "vào" khách khác cũng không (phải làm từ khách nền tảng).
        let operating = Principal::new(Uuid::now_v7(), ALL.map(str::to_owned))
            .operating(TenantId::from_uuid(acme.id));
        assert_eq!(
            autoxide_kernel::tenancy::require_platform(&operating).unwrap_err().code(),
            "tenant_platform_only"
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khach_bi_khoa_nhan_403() {
    TestDatabase::run(async |test| {
        let setup = setup(test, TenancyMode::Multi).await;
        let acme = setup.admin.create(Actor::system(), new_tenant("acme")).await.unwrap();
        let token = format!("tenant:{}", acme.id);
        let path = format!("/api/v1/kernel/tenants/{}", acme.id);
        let (status, read) = call(&setup.app, Method::GET, &path, "platform", None).await;
        assert_eq!(status, StatusCode::OK);
        // Khoá (kèm phiên bản vừa đọc).
        let changes = json!({"status": "suspended"});
        let request = Request::patch(&path)
            .header(header::AUTHORIZATION, "Bearer platform")
            .header(header::CONTENT_TYPE, "application/json")
            .header(
                header::IF_MATCH,
                format!(
                    "\"{}\"",
                    Timestamp::parse_rfc3339(read["data"]["updated_at"].as_str().unwrap())
                        .unwrap()
                        .as_unix_micros()
                ),
            )
            .body(Body::from(changes.to_string()))
            .unwrap();
        let response = setup.app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let (status, body) =
            call(&setup.app, Method::GET, "/api/v1/kernel/tenant", &token, None).await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("tenant_suspended"))
        );
        // Mở lại.
        let resumed = setup
            .admin
            .update(
                Actor::system(),
                acme.id,
                serde_json::from_value::<TenantChanges>(json!({"status": "active"})).unwrap(),
                None,
            )
            .await
            .unwrap();
        assert_eq!(resumed.status, "active");
        let (status, _) =
            call(&setup.app, Method::GET, "/api/v1/kernel/tenant", &token, None).await;
        assert_eq!(status, StatusCode::OK);
        // Khách nền tảng không khoá được.
        let refused = setup
            .admin
            .update(
                Actor::system(),
                DEFAULT_TENANT.as_uuid(),
                serde_json::from_value::<TenantChanges>(json!({"status": "suspended"})).unwrap(),
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(refused.code(), "validation_failed");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn che_do_mot_khach_khong_tao_duoc_khach() {
    TestDatabase::run(async |test| {
        let setup = setup(test, TenancyMode::Single).await;
        let (status, body) = call(
            &setup.app,
            Method::POST,
            "/api/v1/kernel/tenants",
            "platform",
            Some(json!({"slug": "acme", "name": "Acme"})),
        )
        .await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::CONFLICT, Some("tenant_single_mode"))
        );
        let (status, body) =
            call(&setup.app, Method::GET, "/api/v1/kernel/tenant", "platform", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            (body["data"]["slug"].as_str(), body["data"]["mode"].as_str()),
            (Some("default"), Some("single"))
        );
        assert_eq!(setup.seed.runs.load(Ordering::SeqCst), 0);
    })
    .await
    .unwrap();
}

/// Đổi trạng thái kèm lý do: lý do nằm trong event `kernel.tenant_updated` (nhật ký audit); lý do
/// quá dài bị từ chối.
#[tokio::test]
async fn ly_do_doi_trang_thai_vao_audit() {
    TestDatabase::run(async |test| {
        let setup = setup(test, TenancyMode::Multi).await;
        let acme = setup.admin.create(Actor::system(), new_tenant("acme")).await.unwrap();
        let changes = json!({"status": "suspended", "reason": "  Unpaid invoices  "});
        let (status, body) = call(
            &setup.app,
            Method::PATCH,
            &format!("/api/v1/kernel/tenants/{}", acme.id),
            "platform",
            Some(changes),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let tenant = setup
            .registry
            .tenant(autoxide_kernel::tenancy::TenantId::from_uuid(acme.id))
            .await
            .unwrap();
        use sea_orm::ConnectionTrait as _;
        use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Query};
        let select = Query::select()
            .column(Alias::new("payload"))
            .from(Alias::new("kernel_events"))
            .and_where(Expr::col(Alias::new("tenant_id")).eq(acme.id))
            .and_where(Expr::col(Alias::new("name")).eq("kernel.tenant_updated"))
            .to_owned();
        let rows = tenant.database().read().query_all(&select).await.unwrap();
        assert_eq!(rows.len(), 1);
        let payload: Value =
            serde_json::from_str(&rows[0].try_get::<String>("", "payload").unwrap()).unwrap();
        assert_eq!(payload["reason"], "Unpaid invoices", "{payload}");
        assert_eq!(payload["after"]["status"], "suspended");

        let long = json!({"status": "active", "reason": "x".repeat(501)});
        let (status, body) = call(
            &setup.app,
            Method::PATCH,
            &format!("/api/v1/kernel/tenants/{}", acme.id),
            "platform",
            Some(long),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body["errors"][0]["field"], "reason");
    })
    .await
    .unwrap();
}
