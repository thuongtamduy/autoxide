//! Chế độ "nghỉ" (bước 70 P9, D-20261125-dormant-modules): module có trong chương trình nhưng đang tắt
//! vẫn góp phần vào bản xuất dữ liệu cá nhân, vẫn xoá dữ liệu khi tài khoản bị xoá, và bảng của nó đi
//! theo khách khi xoá hẳn / dời khách. Database chưa từng có bảng của nó thì không có gì để làm (và không
//! lỗi). Chạy trên cả SQLite và Postgres.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Delivered, Event, EventHandler, EventInfo, Outbox};
use autoxide_kernel::i18n::Language;
use autoxide_kernel::migration::prelude::*;
use autoxide_kernel::module::{DormantContext, Module, Modules, StartContext};
use autoxide_kernel::privacy::{PersonalData, USER_DELETED, deleted_user};
use autoxide_kernel::tenancy::{TenantContext, TenantId, TenantMoves};
use autoxide_testkit::{TestApp, TestDatabase};
use sea_orm::sea_query::{Alias, Expr, Query};
use serde::Serialize;
use serde_json::{Value, json};
use uuid::Uuid;

/// Module `legacy` có trong chương trình nhưng đang tắt, với một bảng ghi chú theo người dùng từ hồi nó
/// còn chạy.
struct Legacy;

struct CreateLegacyNotes;

impl MigrationName for CreateLegacyNotes {
    fn name(&self) -> &str {
        "m0001_create_legacy_notes"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateLegacyNotes {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("legacy_notes")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("id"))
                    .col(schema::uuid("user_id"))
                    .primary_key(Index::create().col("tenant_id").col("id"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("legacy_notes").to_owned()).await
    }
}

/// Phần dữ liệu cá nhân của `legacy`: số ghi chú của người đó; xoá tài khoản thì xoá ghi chú.
struct LegacyPrivacy;

#[async_trait::async_trait]
impl PersonalData for LegacyPrivacy {
    async fn export(&self, tenant: &TenantContext, user_id: Uuid) -> Result<Value, Error> {
        Ok(json!({"notes": notes(tenant, user_id).await}))
    }
}

#[async_trait::async_trait]
impl EventHandler for LegacyPrivacy {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let Some(user) = deleted_user(event) else { return Ok(()) };
        let delete = Query::delete()
            .from_table(Alias::new("legacy_notes"))
            .and_where(Expr::col(Alias::new("tenant_id")).eq(event.tenant.id().as_uuid()))
            .and_where(Expr::col(Alias::new("user_id")).eq(user))
            .to_owned();
        event.tenant.database().write().execute(&delete).await?;
        Ok(())
    }
}

impl Module for Legacy {
    fn name(&self) -> &'static str {
        "legacy"
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(CreateLegacyNotes)]
    }

    fn start(&mut self, _context: &mut StartContext<'_>) -> Result<(), Error> {
        panic!("a turned-off module never starts");
    }

    // Module tắt không bao giờ được hỏi đăng ký event khác (chỉ `USER_DELETED` qua `dormant`).
    fn subscriptions(&self) -> Vec<autoxide_kernel::event::Subscription> {
        vec![autoxide_kernel::event::Subscription::new(SIGNED_UP, Arc::new(Unreachable))]
    }

    fn dormant(&self, context: &mut DormantContext<'_>) -> Result<(), Error> {
        context.personal_data(Arc::new(LegacyPrivacy))?;
        context.on_user_deleted(Arc::new(LegacyPrivacy))
    }
}

/// Bên nghe event khác của module tắt: không bao giờ được gọi. Đặt cờ thay vì chỉ panic — dispatcher bắt panic
/// của bên nghe, nên panic một mình không làm test đỏ.
struct Unreachable;

/// Có lần `Unreachable` bị gọi.
static REACHED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[async_trait::async_trait]
impl EventHandler for Unreachable {
    async fn handle(&self, _event: &Delivered) -> Result<(), Error> {
        REACHED.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

const SIGNED_UP: &str = "auth.signed_up";

#[derive(Serialize)]
struct SignedUp {
    id: Uuid,
}

impl Event for SignedUp {
    const NAME: &'static str = SIGNED_UP;
}

/// Module quản lý tài khoản giả: phát `auth.user_deleted` (và `auth.signed_up`).
struct Accounts(Arc<Mutex<Option<Outbox>>>);

const USER_DELETED_INFO: EventInfo = EventInfo::new(USER_DELETED, "An account was deleted.");

#[derive(Serialize)]
struct UserDeleted {
    before: Value,
}

impl Event for UserDeleted {
    const NAME: &'static str = USER_DELETED;
}

impl Module for Accounts {
    fn name(&self) -> &'static str {
        "auth"
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        autoxide_kernel::migration::helper_tables(
            "auth",
            &[autoxide_kernel::migration::HelperTable::Events],
        )
    }

    fn events(&self) -> Vec<EventInfo> {
        vec![USER_DELETED_INFO, EventInfo::new(SIGNED_UP, "An account was created.")]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        *self.0.lock().unwrap() = Some(context.outbox()?);
        Ok(())
    }
}

const PLATFORM: [&str; 4] = [
    "kernel.tenants.move",
    "kernel.tenants.read",
    "kernel.tenants.manage",
    "kernel.tenants.delete",
];

async fn app(test: &TestDatabase) -> (TestApp, Outbox) {
    let outbox = Arc::new(Mutex::new(None));
    let app = TestApp::builder(test)
        .setting("AUTOXIDE_TENANCY_CACHE_SECONDS", "1")
        .module(Accounts(outbox.clone()))
        .module_turned_off(Legacy)
        .tenant("a")
        .tenant("b")
        .placement("spare")
        .user("platform", 1, &PLATFORM)
        .build()
        .await
        .unwrap();
    let outbox = outbox.lock().unwrap().clone().unwrap();
    (app, outbox)
}

/// Bảng của `legacy` ở database chính, như hồi nó còn chạy.
async fn install_legacy(app: &TestApp) {
    let legacy = Modules::new(vec![Box::new(Legacy)]).unwrap().migrator().unwrap();
    legacy.up(app.database()).await.unwrap();
}

async fn add_note(tenant: &TenantContext, user: Uuid) {
    let insert = Query::insert()
        .into_table(Alias::new("legacy_notes"))
        .columns(["tenant_id", "id", "user_id"].map(Alias::new))
        .values_panic([tenant.id().as_uuid().into(), Uuid::now_v7().into(), user.into()])
        .to_owned();
    tenant.database().write().execute(&insert).await.unwrap();
}

async fn notes(tenant: &TenantContext, user: Uuid) -> i64 {
    count(tenant.database(), tenant.id(), Some(user)).await
}

async fn count(
    database: &autoxide_kernel::database::Database,
    tenant: TenantId,
    user: Option<Uuid>,
) -> i64 {
    let mut select = Query::select();
    select
        .expr_as(Expr::cust("COUNT(*)"), Alias::new("n"))
        .from(Alias::new("legacy_notes"))
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.as_uuid()));
    if let Some(user) = user {
        select.and_where(Expr::col(Alias::new("user_id")).eq(user));
    }
    let row = database.write().query_one(&select).await.unwrap().unwrap();
    row.try_get("", "n").unwrap()
}

async fn delete_account(app: &TestApp, outbox: &Outbox, tenant: &TenantContext, user: Uuid) {
    let event = UserDeleted { before: json!({"id": user}) };
    outbox.record(tenant.database().write(), tenant, Actor::system(), None, &event).await.unwrap();
    app.modules().dispatcher().unwrap().run_once().await.unwrap();
}

#[tokio::test]
async fn module_nghi_van_xuat_va_xoa_du_lieu_ca_nhan() {
    TestDatabase::run(async |test| {
        let (app, outbox) = app(test).await;
        assert_eq!(app.modules().inactive()[0].name, "legacy");
        let registry = app.modules().personal_data();
        let parts = registry.modules().unwrap();
        assert!(parts.contains(&"legacy"), "{parts:?}");
        let a = app.tenant("a").await.unwrap();
        let (ann, bob) = (Uuid::now_v7(), Uuid::now_v7());

        // Database chưa từng có bảng của `legacy`: bản xuất không có phần của nó, xoá tài khoản vẫn
        // giao xong event (không lỗi, không thử lại mãi).
        let exported = registry.export(&a, ann).await.unwrap();
        assert!(!exported.contains_key("legacy"), "{exported:?}");
        delete_account(&app, &outbox, &a, ann).await;
        let pending = Query::select()
            .expr_as(Expr::cust("COUNT(*)"), Alias::new("n"))
            .from(Alias::new("auth_events"))
            .and_where(Expr::col(Alias::new("delivered_at")).is_null())
            .to_owned();
        let row = app.database().write().query_one(&pending).await.unwrap().unwrap();
        assert_eq!(row.try_get::<i64>("", "n").unwrap(), 0);

        // Hồi `legacy` còn chạy: ghi chú của Ann và Bob.
        install_legacy(&app).await;
        add_note(&a, ann).await;
        add_note(&a, ann).await;
        add_note(&a, bob).await;
        let exported = registry.export(&a, ann).await.unwrap();
        assert_eq!(exported["legacy"], json!({"notes": 2}), "{exported:?}");

        // Xoá tài khoản của Ann: module đang tắt vẫn xoá phần của Ann, không chạm Bob.
        delete_account(&app, &outbox, &a, ann).await;
        assert_eq!(notes(&a, ann).await, 0);
        assert_eq!(notes(&a, bob).await, 1);

        // Event khác (không phải xoá tài khoản) không tới module tắt: `Unreachable` không bị gọi.
        let signed = SignedUp { id: Uuid::now_v7() };
        outbox.record(a.database().write(), &a, Actor::system(), None, &signed).await.unwrap();
        app.modules().dispatcher().unwrap().run_once().await.unwrap();
        assert!(
            !REACHED.load(std::sync::atomic::Ordering::SeqCst),
            "a turned-off module got an event"
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn xoa_han_khach_xoa_ca_bang_cua_module_nghi() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        install_legacy(&app).await;
        let (a, b) = (app.tenant("a").await.unwrap(), app.tenant("b").await.unwrap());
        add_note(&a, Uuid::now_v7()).await;
        add_note(&b, Uuid::now_v7()).await;

        let id = a.id().as_uuid();
        let suspend = json!({"status": "suspended", "reason": "Closing"});
        let answer = app
            .patch(&format!("/api/v1/kernel/tenants/{id}"))
            .token("platform")
            .json(suspend)
            .send()
            .await
            .unwrap();
        assert_eq!(answer.status(), 200, "{}", answer.json());
        let body = json!({"confirm": "a", "reason": "Asked to close the account"});
        let answer = app
            .post(&format!("/api/v1/kernel/tenants/{id}/erase"))
            .token("platform")
            .json(body)
            .send()
            .await
            .unwrap();
        assert_eq!(answer.status(), 200, "{}", answer.json());
        let erased = answer.json()["data"].clone();
        let tables: Vec<&str> = erased["tables"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t["table"].as_str())
            .collect();
        assert!(tables.contains(&"legacy_notes"), "{erased}");
        assert_eq!(count(app.database(), a.id(), None).await, 0);
        assert_eq!(count(app.database(), b.id(), None).await, 1);
    })
    .await
    .unwrap();
}

/// Lượt dời chép cả bảng của module đang tắt: migration của nó được áp ở đích trước khi chép (đích chưa
/// từng có bảng đó), dữ liệu không bị để lại ở nguồn.
#[tokio::test]
async fn doi_khach_mang_theo_bang_cua_module_nghi() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        install_legacy(&app).await;
        let a = app.tenant("a").await.unwrap();
        let main = a.database().clone();
        let (ann, bob) = (Uuid::now_v7(), Uuid::now_v7());
        add_note(&a, ann).await;
        add_note(&a, bob).await;
        let b = app.tenant("b").await.unwrap();
        add_note(&b, ann).await;

        let moves = app.api::<TenantMoves>().unwrap();
        let plan = moves.plan("a", "spare", false, Language::En).await.unwrap();
        assert!(plan.ready, "{plan:?}");
        let legacy = plan.tables.iter().find(|table| table.table == "legacy_notes");
        assert_eq!(legacy.map(|table| table.rows), Some(2), "{plan:?}");

        let started = moves.start(Actor::system(), "a", "spare", false, false).await.unwrap();
        app.skip_move_wait().await.unwrap();
        let done = moves.run(started.id, "test").await.unwrap();
        assert_eq!(done.state, "done", "{done:?}");
        app.registry().invalidate(a.id());
        let a = app.tenant("a").await.unwrap();
        let spare = a.database().clone();
        assert_eq!(count(&spare, a.id(), None).await, 2);
        assert_eq!(count(&main, a.id(), None).await, 0);
        assert_eq!(count(&main, b.id(), None).await, 1);
    })
    .await
    .unwrap();
}

/// Database của placement `name`.
async fn placement(app: &TestApp, name: &str) -> autoxide_kernel::database::Database {
    let select = Query::select()
        .column(Alias::new("id"))
        .from(Alias::new("kernel_placements"))
        .and_where(Expr::col(Alias::new("name")).eq(name))
        .to_owned();
    let row = app.database().write().query_one(&select).await.unwrap().unwrap();
    let id: Uuid = row.try_get("", "id").unwrap();
    let id = autoxide_kernel::tenancy::PlacementId::from_uuid(id);
    app.registry().placement_database(id).await.unwrap()
}

/// Chiều ngược: nguồn chưa từng có bảng của module tắt, đích thì có (placement cũ). Kế hoạch (dry run) không
/// đổi schema và không lỗi; lượt chạy thật tạo bảng ở nguồn rồi dời bình thường.
#[tokio::test]
async fn doi_khach_toi_noi_da_co_bang_cua_module_nghi() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        let spare = placement(&app, "spare").await;
        let legacy = Modules::new(vec![Box::new(Legacy)]).unwrap().migrator().unwrap();
        legacy.up(&spare).await.unwrap();
        let has_table = |database: autoxide_kernel::database::Database| async move {
            autoxide_testkit::schema_snapshot(database.write())
                .await
                .unwrap()
                .table_names()
                .contains(&"legacy_notes")
        };
        assert!(!has_table(app.database().clone()).await);

        let moves = app.api::<TenantMoves>().unwrap();
        let plan = moves.plan("a", "spare", false, Language::En).await.unwrap();
        assert!(plan.ready, "{plan:?}");
        assert!(!has_table(app.database().clone()).await, "a dry run changes no schema");
        let started = moves.start(Actor::system(), "a", "spare", false, false).await.unwrap();
        app.skip_move_wait().await.unwrap();
        let done = moves.run(started.id, "test").await.unwrap();
        assert_eq!(done.state, "done", "{done:?}");
    })
    .await
    .unwrap();
}

/// `legacy` bản mới: thêm cột `title`.
struct LegacyV2;

struct AddTitle;

impl MigrationName for AddTitle {
    fn name(&self) -> &str {
        "m0002_add_title"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for AddTitle {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("legacy_notes")
                    .add_column(schema::string_len_null("title", 100))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(Table::alter().table("legacy_notes").drop_column("title").to_owned())
            .await
    }
}

impl Module for LegacyV2 {
    fn name(&self) -> &'static str {
        "legacy"
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(CreateLegacyNotes), Box::new(AddTitle)]
    }
}

/// Nâng cấp khi module đang tắt: `migrate up` áp migration mới của nó ở database đã có bảng của nó (phần
/// xuất / xoá chạy trên schema mới), không tạo bảng ở database chưa từng có.
#[tokio::test]
async fn nang_cap_ap_migration_cua_module_nghi_noi_no_da_chay() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test)
            .module_turned_off(LegacyV2)
            .placement("spare")
            .build()
            .await
            .unwrap();
        let old = Modules::new(vec![Box::new(Legacy)]).unwrap().migrator().unwrap();
        old.up(app.database()).await.unwrap();
        let migrator = app.modules().migrator().unwrap();
        let pending = migrator.pending(app.database()).await.unwrap();
        assert!(
            pending.iter().any(|state| state.module == "legacy" && state.name == "m0002_add_title"),
            "{pending:?}"
        );
        migrator.up(app.database()).await.unwrap();
        assert!(migrator.pending(app.database()).await.unwrap().is_empty());
        let columns = autoxide_testkit::schema_snapshot(app.database().write()).await.unwrap();
        assert!(format!("{columns:?}").contains("title"), "the new column exists");

        // Database chưa từng có bảng của `legacy`: không có gì chờ, `up` không tạo bảng.
        let spare = placement(&app, "spare").await;
        let pending = migrator.pending(&spare).await.unwrap();
        assert!(pending.iter().all(|state| state.module != "legacy"), "{pending:?}");
        migrator.up(&spare).await.unwrap();
        let tables = autoxide_testkit::schema_snapshot(spare.write()).await.unwrap();
        assert!(!tables.table_names().contains(&"legacy_notes"));
    })
    .await
    .unwrap();
}

/// Gỡ hẳn module tắt: `migrate down` hết migration của nó để lại bảng lịch sử rỗng — module coi như chưa cài:
/// không còn gì chờ (máy chủ khởi động được), `up` không tạo lại bảng, phần "nghỉ" không chạy ở đó.
#[tokio::test]
async fn go_han_module_nghi_bang_migrate_down() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test).module_turned_off(LegacyV2).build().await.unwrap();
        let old = Modules::new(vec![Box::new(Legacy)]).unwrap().migrator().unwrap();
        old.up(app.database()).await.unwrap();
        let migrator = app.modules().migrator().unwrap();
        migrator.up(app.database()).await.unwrap();
        migrator.down(app.database(), "legacy", 1000).await.unwrap();

        let tables = |app: &TestApp| {
            let database = app.database().clone();
            async move {
                autoxide_testkit::schema_snapshot(database.write())
                    .await
                    .unwrap()
                    .table_names()
                    .into_iter()
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            }
        };
        assert!(!tables(&app).await.contains(&"legacy_notes".to_owned()));
        let pending = migrator.pending(app.database()).await.unwrap();
        assert!(pending.iter().all(|state| state.module != "legacy"), "{pending:?}");
        migrator.up(app.database()).await.unwrap();
        assert!(
            !tables(&app).await.contains(&"legacy_notes".to_owned()),
            "up does not recreate it"
        );
        let (dormant, _) = app.modules().dormant_everywhere().await.unwrap();
        assert!(dormant.is_empty(), "{dormant:?}");
    })
    .await
    .unwrap();
}
