//! Placement (bước 33 A9): URL kết nối lưu mã hoá; placement chưa đủ migration thì chỉ khách trên
//! nó nhận 503 (và job của nó đợi); `migrate up --all` hỏng giữa chừng thì chạy lại tiếp từ chỗ
//! dừng. Chạy trên cả SQLite và Postgres.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_kernel::config::{Config, ConfigReader, ConfigSource, DatabaseConfig};
use autoxide_kernel::database::Database;
use autoxide_kernel::event::Actor;
use autoxide_kernel::migration::prelude::*;
use autoxide_kernel::module::{Module, Modules};
use autoxide_kernel::tenancy::{
    DatabaseRegistry, NewPlacement, NewTenant, PlacementKind, Placements, SchemaState,
    TenancyConfig, TenancyMode, TenantAdmin, TenantId, ensure_serving,
};
use autoxide_testkit::TestDatabase;
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Expr, Query};
use serde_json::json;

/// `shop_item`, rồi `shop_order` — migration thứ hai hỏng nếu database có bảng `shop_blocker`
/// (như một placement có dữ liệu lạ làm migration chết giữa chừng).
struct CreateItem;

impl MigrationName for CreateItem {
    fn name(&self) -> &str {
        "m0001_create_item"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateItem {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(Table::create().table("shop_item").col(schema::pk_uuid("id")).to_owned())
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("shop_item").to_owned()).await
    }
}

struct CreateOrder;

impl MigrationName for CreateOrder {
    fn name(&self) -> &str {
        "m0002_create_order"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateOrder {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if manager.has_table("shop_blocker").await? {
            return Err(DbErr::Custom("shop_blocker is in the way".to_owned()));
        }
        manager
            .create_table(Table::create().table("shop_order").col(schema::pk_uuid("id")).to_owned())
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("shop_order").to_owned()).await
    }
}

struct Shop;

impl Module for Shop {
    fn name(&self) -> &'static str {
        "shop"
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(CreateItem), Box::new(CreateOrder)]
    }
}

struct Setup {
    modules: Modules,
    registry: DatabaseRegistry,
    placements: Placements,
    admin: std::sync::Arc<TenantAdmin>,
}

async fn setup(test: &TestDatabase) -> Setup {
    let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
    let mut reader = ConfigReader::new(&source);
    let config = Config::read(&mut reader);
    let mut modules = Modules::new(vec![Box::new(Shop)]).unwrap();
    let configured = modules.configure(&mut reader);
    let config = reader.finish(config.and_then(|config| configured.map(|()| config))).unwrap().0;
    let database = Database::connect(&config.database).await.unwrap();
    modules.migrator().unwrap().up(&database).await.unwrap();
    let registry = DatabaseRegistry::new(
        database,
        config.database,
        TenancyConfig::with_mode(TenancyMode::Multi),
    );
    modules.start_in(&registry).unwrap();
    let placements = modules.placements(&registry).unwrap();
    let admin = modules.api::<TenantAdmin>().unwrap();
    Setup { modules, registry, placements, admin }
}

async fn connect(url: &str) -> Database {
    let source = ConfigSource::from_pairs([("DATABASE_URL", url)]);
    let mut reader = ConfigReader::new(&source);
    let config = DatabaseConfig::read(&mut reader);
    Database::connect(&reader.finish(config).unwrap().0).await.unwrap()
}

fn new_tenant(slug: &str, placement: &str) -> NewTenant {
    serde_json::from_value(json!({"slug": slug, "name": slug, "placement": placement})).unwrap()
}

async fn stored(setup: &Setup, column: &str, name: &str) -> String {
    let select = Query::select()
        .column(Alias::new(column))
        .from(Alias::new("kernel_placements"))
        .and_where(Expr::col(Alias::new("name")).eq(name))
        .to_owned();
    let row = setup.registry.control().read().query_one(&select).await.unwrap().unwrap();
    row.try_get("", column).unwrap()
}

#[tokio::test]
async fn placement_chua_migrate_chi_khach_tren_do_503() {
    TestDatabase::run(async |test| {
        let setup = setup(test).await;
        let url = test.extra_database().await.unwrap();

        // Thêm placement: database trống nên `behind`; URL lưu mã hoá, không còn chữ thường.
        let added = setup
            .placements
            .add(NewPlacement::new("p1", PlacementKind::Shared, url.parse().unwrap()))
            .await
            .unwrap();
        assert_eq!((added.state, added.pending > 0), (SchemaState::Behind, true), "{added:?}");
        let sealed = stored(&setup, "url_sealed", "p1").await;
        assert!(sealed.starts_with("sealed:v1:"), "{sealed}");
        assert!(!sealed.contains(&url));
        let again = setup
            .placements
            .add(NewPlacement::new("p1", PlacementKind::Shared, url.parse().unwrap()))
            .await
            .unwrap_err();
        assert_eq!(again.code(), "tenant_placement_taken");

        // Khách mới không được tạo trên placement thiếu bảng.
        let refused =
            setup.admin.create(Actor::system(), new_tenant("acme", "p1")).await.unwrap_err();
        assert_eq!(refused.code(), "tenant_placement_behind");

        // Migrate mọi placement: tạo được khách trên p1 (mở bằng URL đã mã hoá).
        setup.placements.up_all().await.unwrap();
        let acme = setup.admin.create(Actor::system(), new_tenant("acme", "p1")).await.unwrap();
        let beta = setup.admin.create(Actor::system(), new_tenant("beta", "main")).await.unwrap();
        let acme_id = TenantId::from_uuid(acme.id);
        let acme_context = setup.registry.tenant(acme_id).await.unwrap();
        assert!(ensure_serving(&acme_context).is_ok());
        let p1 = setup.registry.placement_database(acme_context.placement()).await.unwrap();

        // Bản mới có migration p1 chưa chạy (giả lập bằng gỡ một migration ở p1): sau khi kiểm,
        // khách trên p1 nhận 503, khách ở `main` vẫn chạy; job của p1 đợi.
        setup.modules.migrator().unwrap().down(&p1, "shop", 1).await.unwrap();
        let checks = setup.placements.check().await.unwrap();
        assert_eq!(
            checks.iter().map(|check| (check.name.as_str(), check.state)).collect::<Vec<_>>(),
            [("p1", SchemaState::Behind)]
        );
        let behind = setup.registry.tenant(acme_id).await.unwrap();
        let error = ensure_serving(&behind).unwrap_err();
        assert_eq!((error.code(), error.kind().status()), ("tenant_maintenance", 503));
        let other = setup.registry.tenant(TenantId::from_uuid(beta.id)).await.unwrap();
        assert!(ensure_serving(&other).is_ok());
        let names: Vec<String> = setup
            .registry
            .databases()
            .await
            .unwrap()
            .into_iter()
            .map(|(_, name, _)| name)
            .collect();
        assert_eq!(names, ["main"]);

        // `migrate up --all`: p1 đủ bảng lại, khách trên đó chạy lại.
        let report = setup.placements.up_all().await.unwrap();
        let applied: Vec<(&str, usize)> = report
            .iter()
            .map(|section| (section.name.as_str(), section.migrations.len()))
            .collect();
        assert_eq!(applied, [("main", 0), ("p1", 1)]);
        assert!(ensure_serving(&setup.registry.tenant(acme_id).await.unwrap()).is_ok());
        assert_eq!(stored(&setup, "schema_state", "p1").await, "ready");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn migrate_all_chay_lai_tu_cho_dung() {
    TestDatabase::run(async |test| {
        let setup = setup(test).await;
        let first = test.extra_database().await.unwrap();
        let second = test.extra_database().await.unwrap();
        for (name, url) in [("p1", &first), ("p2", &second)] {
            let new = NewPlacement::new(name, PlacementKind::Dedicated, url.parse().unwrap());
            setup.placements.add(new).await.unwrap();
        }
        // p1 có thứ làm migration thứ hai chết; p1 đứng trước p2 theo tên.
        let blocked = connect(&first).await;
        let blocker = Table::create().table("shop_blocker").col(schema::pk_uuid("id")).to_owned();
        SchemaManager::new(blocked.write()).create_table(blocker).await.unwrap();

        // p1 hỏng không chặn p2: p2 vẫn được nâng, lỗi nêu đúng p1.
        let error = setup.placements.up_all().await.unwrap_err().to_string();
        assert!(error.contains("placement `p1`"), "{error}");
        assert!(!error.contains("p2"), "{error}");
        assert!(error.contains("migrate up --all"), "{error}");
        assert_eq!(stored(&setup, "schema_state", "p1").await, "behind");
        assert_eq!(stored(&setup, "schema_state", "p2").await, "ready");
        let lock = setup.placements.lock_main(std::time::Duration::from_secs(60)).await.unwrap();
        let report = setup.placements.up_all_report(&lock).await.unwrap();
        lock.release().await.unwrap();
        let done: Vec<&str> = report.done.iter().map(|section| section.name.as_str()).collect();
        let failed: Vec<&str> = report.failed.iter().map(|failure| failure.name.as_str()).collect();
        assert_eq!((done, failed), (vec!["main", "p2"], vec!["p1"]));

        // Gỡ chỗ hỏng rồi chạy lại: `main` và p1 không còn gì, p2 chạy nốt phần còn thiếu.
        let drop = Table::drop().table("shop_blocker").to_owned();
        SchemaManager::new(blocked.write()).drop_table(drop).await.unwrap();
        let report = setup.placements.up_all().await.unwrap();
        let applied: Vec<(&str, Vec<String>)> = report
            .iter()
            .map(|section| {
                let names = section.migrations.iter().map(|m| m.name.clone()).collect();
                (section.name.as_str(), names)
            })
            .collect();
        assert_eq!(
            applied,
            [("main", vec![]), ("p1", vec!["m0002_create_order".to_owned()]), ("p2", vec![])]
        );
        for section in setup.placements.status_all().await.unwrap() {
            assert!(section.migrations.iter().all(|m| m.applied), "{}", section.name);
        }

        // Kiểm định kỳ không đè trạng thái `migrating` của một lần migrate đang chạy.
        let update = Query::update()
            .table(Alias::new("kernel_placements"))
            .value(Alias::new("schema_state"), "migrating")
            .and_where(Expr::col(Alias::new("name")).eq("p1"))
            .to_owned();
        setup.registry.control().write().execute(&update).await.unwrap();
        let checks = setup.placements.check().await.unwrap();
        assert_eq!(checks[0].state, SchemaState::Migrating);
        assert_eq!(checks[1].state, SchemaState::Ready);
        let _ = blocked.close().await;
    })
    .await
    .unwrap();
}

/// Hai placement, mỗi cái thiếu migration cuối của `shop` (giả lập bản mới).
async fn two_behind(test: &TestDatabase, setup: &Setup) {
    for name in ["p1", "p2"] {
        let url = test.extra_database().await.unwrap();
        let new = NewPlacement::new(name, PlacementKind::Dedicated, url.parse().unwrap());
        setup.placements.add(new).await.unwrap();
    }
    setup.placements.up_all().await.unwrap();
    for (id, _, database) in setup.registry.databases().await.unwrap() {
        if id != autoxide_kernel::tenancy::MAIN_PLACEMENT {
            setup.modules.migrator().unwrap().down(&database, "shop", 1).await.unwrap();
        }
    }
    setup.placements.check().await.unwrap();
}

async fn runs(setup: &Setup) -> Vec<(String, Option<String>, Option<String>)> {
    let select = Query::select()
        .columns([Alias::new("status"), Alias::new("error"), Alias::new("active_key")])
        .from(Alias::new("kernel_migration_runs"))
        .order_by(Alias::new("id"), sea_orm::Order::Asc)
        .to_owned();
    let rows = setup.registry.control().read().query_all(&select).await.unwrap();
    rows.into_iter()
        .map(|row| {
            (
                row.try_get("", "status").unwrap(),
                row.try_get("", "error").unwrap(),
                row.try_get("", "active_key").unwrap(),
            )
        })
        .collect()
}

/// Mọi đường migrate placement cùng giữ khoá migration và chỗ `active` (D-20261126-placement-migration-exclusive):
/// ba lần chạy cùng lúc không đụng nhau — không lần nào hỏng vì bên kia đang tạo cùng bảng, mỗi migration áp
/// dụng đúng một lần, và mỗi lần là một lượt ghi trong `kernel_migration_runs`.
#[tokio::test]
async fn nhieu_duong_migrate_cung_luc_khong_dung_nhau() {
    TestDatabase::run(async |test| {
        let setup = setup(test).await;
        two_behind(test, &setup).await;
        let before = runs(&setup).await.len();

        let (first, second, third) = tokio::join!(
            setup.placements.up_all(),
            setup.placements.up_all(),
            setup.placements.up_placements(Some("p1")),
        );
        let applied: usize = [first.unwrap(), second.unwrap(), third.unwrap()]
            .iter()
            .flatten()
            .filter(|section| section.name != "main")
            .map(|section| section.migrations.len())
            .sum();
        assert_eq!(applied, 2, "mỗi placement nhận migration còn thiếu đúng một lần");
        assert_eq!(stored(&setup, "schema_state", "p1").await, "ready");
        assert_eq!(stored(&setup, "schema_state", "p2").await, "ready");
        let after = runs(&setup).await;
        assert_eq!(after.len(), before + 3, "{after:?}");
        assert!(
            after.iter().all(|(status, _, active)| status == "done" && active.is_none()),
            "{after:?}"
        );
    })
    .await
    .unwrap();
}

/// Tiến trình chết giữa lượt: dòng `running` còn giữ chỗ `active`, placement kẹt `migrating`. Lần chạy sau (đang
/// giữ khoá, nên biết chủ lượt kia đã chết) đánh dấu lượt cũ `failed` và nâng nốt placement.
#[tokio::test]
async fn luot_bo_do_sau_khi_tien_trinh_chet_duoc_don() {
    TestDatabase::run(async |test| {
        let setup = setup(test).await;
        two_behind(test, &setup).await;
        let now = autoxide_kernel::time::Timestamp::now();
        let insert = Query::insert()
            .into_table(Alias::new("kernel_migration_runs"))
            .columns(
                ["id", "placement", "status", "result", "created_at", "updated_at", "active_key"]
                    .map(Alias::new),
            )
            .values_panic([
                uuid::Uuid::now_v7().into(),
                Option::<String>::None.into(),
                "running".into(),
                "[]".into(),
                now.into(),
                now.into(),
                "active".into(),
            ])
            .to_owned();
        setup.registry.control().write().execute(&insert).await.unwrap();
        let stuck = Query::update()
            .table(Alias::new("kernel_placements"))
            .value(Alias::new("schema_state"), "migrating")
            .and_where(Expr::col(Alias::new("name")).eq("p1"))
            .to_owned();
        setup.registry.control().write().execute(&stuck).await.unwrap();

        setup.placements.up_all().await.unwrap();

        let found = runs(&setup).await;
        let (status, error, active) = &found[found.len() - 2];
        assert_eq!((status.as_str(), active), ("failed", &None), "{found:?}");
        assert!(error.as_deref().unwrap_or("").starts_with("interrupted"), "{found:?}");
        assert_eq!(found.last().map(|run| run.0.as_str()), Some("done"), "{found:?}");
        assert_eq!(stored(&setup, "schema_state", "p1").await, "ready");
        assert_eq!(stored(&setup, "schema_state", "p2").await, "ready");
    })
    .await
    .unwrap();
}

async fn placements_check(setup: &Setup) -> autoxide_kernel::doctor::Finding {
    let (_, check) = setup
        .modules
        .checks()
        .into_iter()
        .find(|(module, check)| *module == "kernel" && check.name == "placements")
        .unwrap();
    check.run().await
}

/// `doctor` (bước `kernel.placements`) nêu placement chưa sẵn sàng, kể cả placement kẹt `migrating` do tiến trình
/// đã dừng, và chỉ cách sửa; migrate xong thì đạt.
#[tokio::test]
async fn doctor_bao_placement_chua_san_sang() {
    use autoxide_kernel::doctor::Status;

    TestDatabase::run(async |test| {
        let setup = setup(test).await;
        two_behind(test, &setup).await;
        let stuck = Query::update()
            .table(Alias::new("kernel_placements"))
            .value(Alias::new("schema_state"), "migrating")
            .and_where(Expr::col(Alias::new("name")).eq("p2"))
            .to_owned();
        setup.registry.control().write().execute(&stuck).await.unwrap();

        let finding = placements_check(&setup).await;
        assert_eq!(finding.status, Status::Failed, "{}", finding.message);
        assert!(
            finding.message.contains("1 placement(s) behind, their tenants get 503: p1"),
            "{}",
            finding.message
        );
        assert!(
            finding.message.contains("left `migrating` by a process that stopped midway: p2"),
            "{}",
            finding.message
        );
        assert!(finding.hint.as_deref().unwrap_or("").contains("migrate up --all"));

        setup.placements.up_all().await.unwrap();
        let finding = placements_check(&setup).await;
        assert_eq!(
            (finding.status, finding.message.as_str()),
            (Status::Ok, "2 placement(s), all ready.")
        );
    })
    .await
    .unwrap();
}
