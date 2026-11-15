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
    let now = chrono::Utc::now();
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
    let now = chrono::Utc::now();
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
    let now = chrono::Utc::now();
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
