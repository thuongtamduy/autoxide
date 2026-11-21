//! Nền đa khách (bước 33 A1): bảng mặt điều khiển gieo khách mặc định, sổ database ở chế độ
//! `single` (không truy vấn gì) và `multi` (tra theo ID / slug / tên miền đã xác minh, cache có
//! TTL, mở placement lười, giới hạn số pool). Chạy trên cả SQLite và Postgres; placement riêng là
//! file SQLite tạm.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_kernel::config::{Config, ConfigReader, ConfigSource, DatabaseConfig};
use autoxide_kernel::database::Database;
use autoxide_kernel::module::Modules;
use autoxide_kernel::tenancy::{
    DEFAULT_TENANT, DatabaseRegistry, MAIN_PLACEMENT, TenancyConfig, TenancyMode, TenantId,
    TenantStatus, ensure_serving,
};
use autoxide_kernel::time::Timestamp;
use autoxide_testkit::TestDatabase;
use sea_orm::ConnectionTrait as _;
use sea_orm::sea_query::{Alias, ExprTrait as _, Query};
use uuid::Uuid;

/// Database điều khiển đã migrate (module `kernel`), cùng cấu hình của nó.
async fn control(test: &TestDatabase) -> (Database, DatabaseConfig) {
    let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
    let mut reader = ConfigReader::new(&source);
    let config = Config::read(&mut reader);
    let mut modules = Modules::new(Vec::new()).unwrap();
    let configured = modules.configure(&mut reader);
    let (config, _) = reader.finish(config.and_then(|config| configured.map(|_| config))).unwrap();
    let database = Database::connect(&config.database).await.unwrap();
    modules.migrator().unwrap().up(&database).await.unwrap();
    (database, config.database)
}

async fn insert(database: &Database, table: &str, columns: &[&str], values: Vec<sea_orm::Value>) {
    let mut insert = Query::insert();
    insert
        .into_table(Alias::new(table))
        .columns(columns.iter().map(|column| Alias::new(*column)))
        .values_panic(values.into_iter().map(Into::into));
    database.write().execute(&insert).await.unwrap();
}

/// Placement riêng: một file SQLite tạm.
async fn placement(database: &Database, name: &str, dir: &std::path::Path) -> Uuid {
    let id = Uuid::now_v7();
    let url = format!("sqlite://{}", dir.join(format!("{name}.db")).display());
    let now = Timestamp::now();
    insert(
        database,
        "kernel_placements",
        &[
            "id",
            "name",
            "kind",
            "backend",
            "url_sealed",
            "schema_state",
            "created_at",
            "updated_at",
        ],
        vec![
            id.into(),
            name.into(),
            "dedicated".into(),
            "sqlite".into(),
            url.into(),
            "ready".into(),
            now.into(),
            now.into(),
        ],
    )
    .await;
    id
}

async fn tenant(database: &Database, slug: &str, status: &str, placement: Uuid) -> TenantId {
    let id = Uuid::now_v7();
    let now = Timestamp::now();
    insert(
        database,
        "kernel_tenants",
        &["id", "slug", "name", "status", "placement_id", "created_at", "updated_at"],
        vec![
            id.into(),
            slug.into(),
            slug.to_uppercase().into(),
            status.into(),
            placement.into(),
            now.into(),
            now.into(),
        ],
    )
    .await;
    TenantId::from_uuid(id)
}

async fn domain(database: &Database, tenant: TenantId, name: &str, status: &str) {
    let now = Timestamp::now();
    insert(
        database,
        "kernel_tenant_domains",
        &["id", "tenant_id", "domain", "status", "verification_token", "created_at", "updated_at"],
        vec![
            Uuid::now_v7().into(),
            tenant.as_uuid().into(),
            name.into(),
            status.into(),
            "token".into(),
            now.into(),
            now.into(),
        ],
    )
    .await;
}

#[tokio::test]
async fn migration_gieo_khach_mac_dinh_va_single_khong_truy_van_gi() {
    TestDatabase::run(async |test| {
        let (database, _) = control(test).await;
        let row = database
            .read()
            .query_one(
                &Query::select()
                    .columns(["slug", "status", "placement_id"].map(Alias::new))
                    .from(Alias::new("kernel_tenants"))
                    .to_owned(),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get::<String>("", "slug").unwrap(), "default");
        assert_eq!(row.try_get::<Uuid>("", "placement_id").unwrap(), MAIN_PLACEMENT.as_uuid());

        let registry = DatabaseRegistry::single(database.clone());
        let default = registry.tenant(DEFAULT_TENANT).await.unwrap();
        assert!(default.is_platform());
        assert_eq!((default.slug(), default.storage_prefix().as_str()), ("default", ""));
        assert!(ensure_serving(&default).is_ok());
        let other = registry.tenant(TenantId::from_uuid(Uuid::now_v7())).await.unwrap_err();
        assert_eq!(other.code(), "tenant_not_found");
        assert!(registry.tenant_by_domain("shop.example").await.unwrap().is_none());
        assert_eq!(registry.databases().await.unwrap().len(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn multi_tra_theo_id_slug_ten_mien_va_gioi_han_so_pool() {
    TestDatabase::run(async |test| {
        let (database, base) = control(test).await;
        let dir = std::env::temp_dir().join(format!("autoxide-tenancy-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let config = TenancyConfig::with_mode(TenancyMode::Multi).with_max_open_databases(2);
        let registry = DatabaseRegistry::new(database.clone(), base, config);

        let places = [
            placement(&database, "p1", &dir).await,
            placement(&database, "p2", &dir).await,
            placement(&database, "p3", &dir).await,
        ];
        let a = tenant(&database, "a", "active", places[0]).await;
        let b = tenant(&database, "b", "suspended", places[1]).await;
        let c = tenant(&database, "c", "active", places[2]).await;
        let shared = tenant(&database, "s", "active", MAIN_PLACEMENT.as_uuid()).await;
        domain(&database, a, "shop-a.example", "verified").await;
        domain(&database, c, "shop-c.example", "pending").await;

        let found = registry.tenant(a).await.unwrap();
        assert_eq!((found.slug(), found.status()), ("a", TenantStatus::Active));
        assert!(!found.is_platform());
        assert_eq!(found.storage_prefix(), format!("tenants/{a}/"));
        assert_eq!(registry.tenant_by_slug("b").await.unwrap().id(), b);
        assert_eq!(
            ensure_serving(&registry.tenant(b).await.unwrap()).unwrap_err().code(),
            "tenant_suspended"
        );
        // Khách chung database điều khiển dùng chính pool của nó.
        let on_main = registry.tenant(shared).await.unwrap();
        assert_eq!(on_main.placement(), MAIN_PLACEMENT);
        // Tên miền: chỉ tên đã xác minh; chữ hoa và dấu chấm cuối không đổi gì.
        assert_eq!(registry.tenant_by_domain("Shop-A.example.").await.unwrap().unwrap().id(), a);
        assert!(registry.tenant_by_domain("shop-c.example").await.unwrap().is_none());
        assert!(registry.tenant_by_slug("nope").await.is_err());

        // Mỗi placement một database riêng; tối đa 2 pool mở cùng lúc.
        let tenant_c = registry.tenant(c).await.unwrap();
        tenant_c
            .database()
            .write()
            .execute_unprepared("CREATE TABLE only_c (x INTEGER)")
            .await
            .unwrap();
        assert!(registry.open_databases() <= 2);
        let tenant_a = registry.tenant(a).await.unwrap();
        assert!(
            tenant_a.database().read().execute_unprepared("SELECT x FROM only_c").await.is_err(),
            "placements do not share a database"
        );
        assert!(registry.open_databases() <= 2);
        // Sửa khách rồi `invalidate`: đọc lại trạng thái mới ngay.
        database
            .write()
            .execute(
                &Query::update()
                    .table(Alias::new("kernel_tenants"))
                    .value(Alias::new("status"), "maintenance")
                    .and_where(sea_orm::sea_query::Expr::col(Alias::new("slug")).eq("a"))
                    .to_owned(),
            )
            .await
            .unwrap();
        assert_eq!(registry.tenant(a).await.unwrap().status(), TenantStatus::Active, "cached");
        registry.invalidate(a);
        let refreshed = registry.tenant(a).await.unwrap();
        assert_eq!(refreshed.status(), TenantStatus::Maintenance);
        assert_eq!(ensure_serving(&refreshed).unwrap_err().code(), "tenant_maintenance");
        // Worker duyệt mọi placement.
        assert_eq!(registry.databases().await.unwrap().len(), 4);
        let _ = std::fs::remove_dir_all(&dir);
    })
    .await
    .unwrap();
}

// --- Ngữ cảnh khách của request (bước 33 A2) -----------------------------------------------------

mod request {
    use std::collections::HashMap;
    use std::sync::Arc;

    use autoxide_kernel::access::{Authenticator, Principal};
    use autoxide_kernel::config::{ConfigReader, ConfigSource, HttpConfig};
    use autoxide_kernel::error::Error;
    use autoxide_kernel::http;
    use autoxide_kernel::response::Data;
    use autoxide_kernel::tenancy::{
        DatabaseRegistry, MAIN_PLACEMENT, TenancyConfig, TenancyMode, TenantContext, TenantHint,
        TenantId,
    };
    use autoxide_testkit::TestDatabase;
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use serde_json::{Value, json};
    use tower::ServiceExt;
    use utoipa_axum::router::OpenApiRouter;
    use utoipa_axum::routes;
    use uuid::Uuid;

    use super::{control, domain, tenant};

    /// Token → bên gọi (kèm khách của credential).
    struct Tokens(HashMap<&'static str, Principal>);

    #[async_trait::async_trait]
    impl Authenticator for Tokens {
        async fn authenticate(
            &self,
            authorization: Option<&str>,
            _: &TenantHint,
        ) -> Result<Principal, Error> {
            authorization
                .and_then(|value| value.strip_prefix("Bearer "))
                .and_then(|token| self.0.get(token))
                .cloned()
                .ok_or_else(|| Error::unauthenticated("auth.missing_token", "Sign in."))
        }
    }

    /// Khách của request và (nếu đăng nhập) bên gọi.
    #[utoipa::path(get, path = "/", responses((status = 200, description = "Who and where.")))]
    async fn whoami(
        tenant: TenantContext,
        principal: Option<Principal>,
    ) -> Result<Data<Value>, Error> {
        Ok(Data {
            data: json!({
                "tenant": tenant.slug(),
                "operator_from": principal.as_ref().and_then(Principal::operator_from).map(|id| id.to_string()),
            }),
        })
    }

    fn app(registry: DatabaseRegistry, tokens: HashMap<&'static str, Principal>) -> Router {
        let source = ConfigSource::default();
        let mut reader = ConfigReader::new(&source);
        let config = HttpConfig::read(&mut reader);
        let config = reader.finish(config).unwrap().0;
        let routes = OpenApiRouter::new().routes(routes!(whoami));
        let authenticator: Arc<dyn Authenticator> = Arc::new(Tokens(tokens));
        http::build(
            &config,
            OpenApiRouter::new(),
            vec![("probe", routes)],
            http::Extras::default().authenticator(Some(authenticator)).tenancy(registry),
        )
        .router
    }

    async fn call(app: &Router, headers: &[(&str, &str)]) -> (StatusCode, Value) {
        let mut request = Request::builder().uri("/api/v1/probe");
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = app.clone().oneshot(request.body(Body::empty()).unwrap()).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn single_bo_qua_host_va_header() {
        TestDatabase::run(async |test| {
            let (database, _) = control(test).await;
            let tokens = HashMap::from([("a", Principal::new(Uuid::nil(), []))]);
            let app = app(DatabaseRegistry::single(database), tokens);
            for headers in [
                vec![],
                vec![("host", "shop.example"), ("autoxide-tenant", "other")],
                vec![("authorization", "Bearer a"), ("autoxide-tenant", "other")],
            ] {
                let (status, body) = call(&app, &headers).await;
                assert_eq!(
                    (status, body["data"]["tenant"].as_str()),
                    (StatusCode::OK, Some("default")),
                    "{headers:?}"
                );
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn multi_credential_header_host_va_nhan_su_nen_tang() {
        TestDatabase::run(async |test| {
            let (database, base) = control(test).await;
            let main = MAIN_PLACEMENT.as_uuid();
            let a = tenant(&database, "a", "active", main).await;
            let b = tenant(&database, "b", "active", main).await;
            let locked = tenant(&database, "locked", "suspended", main).await;
            domain(&database, b, "shop-b.example", "verified").await;
            let platform = autoxide_kernel::tenancy::DEFAULT_TENANT;
            let user = |tenant: TenantId, permissions: &[&str]| {
                Principal::new(Uuid::now_v7(), permissions.iter().map(|p| (*p).to_owned()))
                    .in_tenant(tenant)
            };
            let tokens = HashMap::from([
                ("a", user(a, &[])),
                ("staff", user(platform, &["kernel.tenants.operate"])),
                ("plain-staff", user(platform, &[])),
                ("admin-a", Principal::with_all_permissions(Uuid::now_v7()).in_tenant(a)),
            ]);
            let config = TenancyConfig::with_mode(TenancyMode::Multi);
            let app = app(DatabaseRegistry::new(database, base, config), tokens);
            let tenant_of = async |headers: &[(&str, &str)]| {
                let (status, body) = call(&app, headers).await;
                (
                    status,
                    body["data"]["tenant"]
                        .as_str()
                        .map(str::to_owned)
                        .or_else(|| body["code"].as_str().map(str::to_owned)),
                )
            };

            // Credential: khách của credential.
            assert_eq!(
                tenant_of(&[("authorization", "Bearer a")]).await,
                (StatusCode::OK, Some("a".to_owned()))
            );
            // Không gì cả: phải nói rõ cách sửa.
            assert_eq!(
                tenant_of(&[]).await,
                (StatusCode::BAD_REQUEST, Some("tenant_required".to_owned()))
            );
            // Route công khai: header hay tên miền đã xác minh chọn khách.
            assert_eq!(tenant_of(&[("autoxide-tenant", "b")]).await.1.as_deref(), Some("b"));
            assert_eq!(
                tenant_of(&[("autoxide-tenant", &b.to_string())]).await.1.as_deref(),
                Some("b")
            );
            assert_eq!(tenant_of(&[("host", "Shop-B.example:443")]).await.1.as_deref(), Some("b"));
            assert_eq!(
                tenant_of(&[("host", "unknown.example")]).await.1.as_deref(),
                Some("tenant_required")
            );
            // Credential của khách A trỏ sang khách B (header hay tên miền): `tenant_mismatch`.
            for headers in [
                [("authorization", "Bearer a"), ("autoxide-tenant", "b")],
                [("authorization", "Bearer a"), ("host", "shop-b.example")],
            ] {
                assert_eq!(
                    tenant_of(&headers).await,
                    (StatusCode::FORBIDDEN, Some("tenant_mismatch".to_owned()))
                );
            }
            // Admin của khách thường có mọi permission nhưng không phải nhân sự nền tảng.
            let (status, body) =
                call(&app, &[("authorization", "Bearer admin-a"), ("autoxide-tenant", "b")]).await;
            assert_eq!(
                (status, body["code"].as_str()),
                (StatusCode::FORBIDDEN, Some("tenant_mismatch"))
            );
            // Nhân sự nền tảng có `kernel.tenants.operate`: vào khách B, ghi lại từ đâu tới.
            let (status, body) =
                call(&app, &[("authorization", "Bearer staff"), ("autoxide-tenant", "b")]).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_eq!(body["data"]["tenant"], "b");
            assert_eq!(body["data"]["operator_from"], platform.to_string());
            // Thiếu permission: không vào được.
            let (status, _) =
                call(&app, &[("authorization", "Bearer plain-staff"), ("autoxide-tenant", "b")])
                    .await;
            assert_eq!(status, StatusCode::FORBIDDEN);
            // Khách tạm khoá; khách không có.
            assert_eq!(
                tenant_of(&[("autoxide-tenant", "locked")]).await.1.as_deref(),
                Some("tenant_suspended")
            );
            let _ = locked;
            assert_eq!(
                tenant_of(&[("autoxide-tenant", "ghost")]).await.1.as_deref(),
                Some("tenant_not_found")
            );
        })
        .await
        .unwrap();
    }
}
