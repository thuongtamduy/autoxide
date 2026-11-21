//! Tách / gộp khách trên chương trình có mọi module (bước 38): mọi bảng cấp khách của mọi module có
//! chính sách; khách `a` có cây tổ chức (`organizations`), sổ cái ở nút `north` (`ledger`, chuỗi niêm
//! phong), audit log và kho báo cáo gộp; tách nhánh `north` thành khách riêng ở database `spare`
//! (khách gốc mới) — sổ đi theo nhánh và chuỗi vẫn kiểm được, audit về nhánh được ghi lại vào chuỗi
//! của khách mới mà chuỗi nguồn không gãy, số liệu báo cáo dời sang kho của khách mới; gộp ngược lại
//! thì mọi bảng của `a` y như trước khi tách. Chạy trên database gate đang chọn.

use autoxide_kernel::access::Principal;
use autoxide_kernel::database::Database;
use autoxide_kernel::doctor::Status;
use autoxide_kernel::event::Actor;
use autoxide_kernel::i18n::Language;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::tenancy::{
    OrgId, RelocationInput, RelocationKind, TenantContext, TenantId, TenantRelocations,
};
use autoxide_kernel::time::{Date, Timestamp};
use autoxide_ledger::api::{AccountKey, Ledger, LineAccount, NewEntry, NewLine};
use autoxide_testkit::{TestApp, TestDatabase};
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Query};
use sea_orm::{ConnectionTrait as _, DatabaseBackend, Statement};
use serde_json::json;
use uuid::Uuid;

async fn app(test: &TestDatabase) -> TestApp {
    let dir = std::env::temp_dir().join(format!("autoxide-relocate-{}", Uuid::now_v7()));
    let mut builder = TestApp::builder(test)
        .setting("AUTOXIDE_FILES_DIR", &dir.join("files").display().to_string())
        .setting("AUTOXIDE_MAIL_FILE_DIR", &dir.join("mail").display().to_string())
        .setting("AUTOXIDE_TENANCY_CACHE_SECONDS", "1")
        .setting("AUTOXIDE_RELOCATION_DRAIN_SECONDS", "10")
        .tenant("a")
        .placement("spare")
        .user("platform", 1, &["kernel.tenants.relocate", "kernel.tenants.read"])
        .principal_in("a", "owner", Principal::with_all_permissions(Uuid::from_u128(2)));
    for module in crate::modules::all() {
        builder = builder.boxed_module(module);
    }
    let app = builder.build().await.unwrap();
    app.skip_relocation_wait().unwrap();
    app
}

async fn tenant(app: &TestApp, slug: &str) -> TenantContext {
    let id = app.tenant(slug).await.unwrap().id();
    app.registry().invalidate(id);
    app.tenant(slug).await.unwrap()
}

/// Giao mọi event, chạy mọi job đến hạn, tới khi không còn gì.
async fn settle(app: &TestApp) {
    let dispatcher = app.modules().dispatcher().unwrap();
    let worker = app.modules().worker(4).unwrap();
    for _ in 0..20 {
        let delivered = dispatcher.run_once().await.unwrap();
        let ran = worker.run_once().await.unwrap();
        if delivered == 0 && ran == 0 {
            return;
        }
    }
}

async fn unit(app: &TestApp, parent: Uuid, name: &str) -> Uuid {
    let body = json!({"parent_id": parent, "unit_type": "store", "name": name});
    let answer = app
        .post("/api/v1/organizations/units")
        .token("owner")
        .header("autoxide-tenant", "a")
        .json(body)
        .send()
        .await
        .unwrap();
    assert_eq!(answer.status(), 201, "{}", answer.text());
    answer.json()["data"]["id"].as_str().unwrap().parse().unwrap()
}

async fn sell(ledger: &dyn Ledger, tenant: &TenantContext, org: Uuid, reference: &str) {
    let org = OrgId::from_uuid(org);
    let amount = Money::new(150_000, Currency::VND);
    let key = |purpose: &str| LineAccount::Key(AccountKey::new(org, purpose, amount.currency()));
    let entry = NewEntry::new(
        org,
        "demo",
        "sale",
        reference,
        "Sale",
        vec![
            NewLine::debit(key("cash.bank"), amount),
            NewLine::credit(key("revenue.sales"), amount),
        ],
    );
    ledger.post(tenant, Actor::system(), entry).await.unwrap();
}

/// Bảng có cột `tenant_id` của `database`.
async fn tenant_tables(database: &Database) -> Vec<String> {
    let backend = database.write().get_database_backend();
    let sql = match backend {
        DatabaseBackend::Sqlite => {
            "SELECT m.name AS name FROM sqlite_master m WHERE m.type = 'table' AND EXISTS \
             (SELECT 1 FROM pragma_table_info(m.name) c WHERE c.name = 'tenant_id') ORDER BY m.name"
        }
        _ => {
            "SELECT table_name::text AS name FROM information_schema.columns WHERE \
             table_schema = current_schema() AND column_name = 'tenant_id' ORDER BY table_name"
        }
    };
    let rows = database.write().query_all_raw(Statement::from_string(backend, sql)).await.unwrap();
    rows.iter().map(|row| row.try_get::<String>("", "name").unwrap()).collect()
}

/// Một giá trị dạng chữ (thử từng kiểu; cùng database thì luôn cùng kết quả).
fn text(row: &sea_orm::QueryResult, column: &str) -> String {
    macro_rules! attempt {
        ($($kind:ty),*) => {
            $(if let Ok(value) = row.try_get::<Option<$kind>>("", column) {
                return format!("{value:?}");
            })*
        };
    }
    attempt!(Uuid, Timestamp, Date, i64, i32, i16, bool, f64, String, Vec<u8>);
    "?".to_owned()
}

/// Mọi dòng của khách `tenant` trong mọi bảng cấp khách của `database` (trừ bảng mà tách / gộp
/// thêm dòng theo thiết kế: audit, hàng đợi, nhật ký request, nhật ký dời kho).
async fn dump(database: &Database, tenant: TenantId) -> Vec<String> {
    let skip = |table: &str| {
        table.starts_with("audit_")
            || table.ends_with("_events")
            || table.ends_with("_jobs")
            || table == "kernel_request_log"
            || table == "kernel_throttle"
            || table == "reporting_rehomes"
            || table.starts_with("kernel_tenant")
            || table == "kernel_relocations"
            || table == "kernel_migration_runs"
    };
    let mut rows = Vec::new();
    for table in tenant_tables(database).await.into_iter().filter(|table| !skip(table)) {
        let select = Query::select()
            .expr(Expr::cust("*"))
            .from(Alias::new(table.as_str()))
            .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.as_uuid()))
            .to_owned();
        for row in database.write().query_all(&select).await.unwrap() {
            let values: Vec<String> = row
                .column_names()
                .iter()
                .map(|column| format!("{column}={}", text(&row, column)))
                .collect();
            rows.push(format!("{table}: {}", values.join(", ")));
        }
    }
    rows.sort();
    rows
}

async fn count(database: &Database, table: &str, condition: (&str, Uuid)) -> i64 {
    let select = Query::select()
        .expr_as(Expr::cust("COUNT(*)"), Alias::new("n"))
        .from(Alias::new(table))
        .and_where(Expr::col(Alias::new(condition.0)).eq(condition.1))
        .to_owned();
    database.write().query_one(&select).await.unwrap().unwrap().try_get("", "n").unwrap()
}

async fn checks_pass(app: &TestApp) {
    for (module, check) in app.modules().checks() {
        if module == "audit" || module == "ledger" {
            let finding = check.run().await;
            assert_eq!(finding.status, Status::Ok, "{module}.{}: {}", check.name, finding.message);
        }
    }
}

#[tokio::test]
async fn split_and_merge_with_every_module() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let service = app.api::<TenantRelocations>().unwrap();
        // Mọi bảng cấp khách của mọi module đã khai chính sách tách / gộp.
        let uncovered = service.uncovered_tables(app.default_tenant().database()).await.unwrap();
        assert!(uncovered.is_empty(), "tables without a relocation policy: {uncovered:?}");

        let a = tenant(&app, "a").await;
        let main = a.database().clone();
        let body = json!({"code": "store", "name": "Store"});
        let answer = app
            .post("/api/v1/organizations/unit-types")
            .token("owner")
            .header("autoxide-tenant", "a")
            .json(body)
            .send()
            .await
            .unwrap();
        assert_eq!(answer.status(), 201, "{}", answer.text());
        let root = a.id().as_uuid();
        let north = unit(&app, root, "North").await;
        let shop = unit(&app, north, "North shop").await;
        let ledger = app.api::<dyn Ledger>().unwrap();
        sell(ledger.as_ref(), &a, north, "N-1").await;
        sell(ledger.as_ref(), &a, shop, "N-2").await;
        sell(ledger.as_ref(), &a, root, "R-1").await;
        settle(&app).await;
        let before = dump(&main, a.id()).await;
        let audit_before = count(&main, "audit_entry", ("tenant_id", root)).await;
        let facts_before = count(&main, "reporting_facts", ("tenant_id", root)).await;
        assert!(facts_before >= 3, "{facts_before}");

        // Tách `north` thành khách gốc riêng ở `spare`.
        let split = RelocationInput {
            kind: RelocationKind::Split,
            tenant: "a".to_owned(),
            org_id: Some(north),
            into: None,
            under_org: None,
            slug: Some("a-north".to_owned()),
            name: Some("A North".to_owned()),
            placement: Some("spare".to_owned()),
            keep_parent: false,
            reason: "The north branch becomes its own company.".to_owned(),
        };
        let plan = serde_json::to_value(service.plan(&split, Language::En).await.unwrap()).unwrap();
        assert_eq!(plan["ready"], json!(true), "{plan}");
        let started = service.start(Actor::system(), &split, None, false).await.unwrap();
        let done = service.run(started.id, "test").await.unwrap();
        assert_eq!(done.state, "done", "{done:?}");
        settle(&app).await;
        let moved = tenant(&app, "a-north").await;
        let spare = moved.database().clone();
        assert_eq!(moved.id().as_uuid(), north);
        // Sổ của hai nút trong nhánh đi theo; chuỗi niêm phong kiểm được ở cả hai database.
        assert_eq!(count(&spare, "ledger_books", ("tenant_id", north)).await, 2);
        assert_eq!(count(&main, "ledger_books", ("tenant_id", root)).await, 1);
        assert_eq!(count(&spare, "organizations_unit", ("tenant_id", north)).await, 2);
        checks_pass(&app).await;
        // Audit về nhánh được ghi lại ở khách mới; nguồn giữ nguyên mọi dòng cũ.
        assert!(count(&spare, "audit_entry", ("tenant_id", north)).await > 0);
        assert!(count(&main, "audit_entry", ("tenant_id", root)).await >= audit_before);
        // Kho báo cáo: số liệu của nhánh sang kho của khách gốc mới.
        assert!(count(&spare, "reporting_facts", ("tenant_id", north)).await >= 2);
        assert_eq!(count(&spare, "reporting_rehomes", ("tenant_id", north)).await, 1);
        let left = count(&main, "reporting_facts", ("tenant_id", root)).await;
        assert!(left < facts_before, "{left} < {facts_before}");

        // Gộp ngược lại dưới gốc của `a`: mọi bảng của `a` như trước khi tách.
        let merge = RelocationInput {
            kind: RelocationKind::Merge,
            tenant: "a-north".to_owned(),
            org_id: None,
            into: Some("a".to_owned()),
            under_org: Some(root),
            slug: None,
            name: None,
            placement: None,
            keep_parent: true,
            reason: "The north branch comes back.".to_owned(),
        };
        let plan = serde_json::to_value(service.plan(&merge, Language::En).await.unwrap()).unwrap();
        assert_eq!(plan["ready"], json!(true), "{plan}");
        let started = service.start(Actor::system(), &merge, None, false).await.unwrap();
        let done = service.run(started.id, "test").await.unwrap();
        assert_eq!(done.state, "done", "{done:?}");
        settle(&app).await;
        let after = dump(&main, a.id()).await;
        let missing: Vec<&String> = before.iter().filter(|row| !after.contains(row)).collect();
        let extra: Vec<&String> = after.iter().filter(|row| !before.contains(row)).collect();
        assert!(missing.is_empty() && extra.is_empty(), "missing: {missing:#?}\nextra: {extra:#?}");
        checks_pass(&app).await;
        let status = app
            .get(&format!("/api/v1/kernel/relocations/{}", started.id))
            .token("platform")
            .send()
            .await
            .unwrap();
        assert_eq!(status.json()["data"]["state"], json!("done"));
    })
    .await
    .unwrap();
}

/// Khách mới vẫn là con của khách nguồn (cùng gốc, cùng kho): số liệu của nhánh đổi khách nguồn tại
/// chỗ trong kho của gốc, tổng theo ngày tính lại hai bên.
#[tokio::test]
async fn reporting_rehomed_after_split_under_the_same_root() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let service = app.api::<TenantRelocations>().unwrap();
        let a = tenant(&app, "a").await;
        let main = a.database().clone();
        let answer = app
            .post("/api/v1/organizations/unit-types")
            .token("owner")
            .header("autoxide-tenant", "a")
            .json(json!({"code": "store", "name": "Store"}))
            .send()
            .await
            .unwrap();
        assert_eq!(answer.status(), 201, "{}", answer.text());
        let root = a.id().as_uuid();
        let north = unit(&app, root, "North").await;
        let ledger = app.api::<dyn Ledger>().unwrap();
        sell(ledger.as_ref(), &a, north, "N-1").await;
        sell(ledger.as_ref(), &a, root, "R-1").await;
        settle(&app).await;
        let total = count(&main, "reporting_facts", ("tenant_id", root)).await;
        let split = RelocationInput {
            kind: RelocationKind::Split,
            tenant: "a".to_owned(),
            org_id: Some(north),
            into: None,
            under_org: None,
            slug: Some("a-north".to_owned()),
            name: Some("A North".to_owned()),
            placement: Some("spare".to_owned()),
            keep_parent: true,
            reason: "The north branch gets its own database.".to_owned(),
        };
        let started = service.start(Actor::system(), &split, None, false).await.unwrap();
        assert_eq!(service.run(started.id, "test").await.unwrap().state, "done");
        settle(&app).await;
        // Kho vẫn ở gốc `a`: cùng số số liệu, phần của nhánh mang khách nguồn mới.
        assert_eq!(count(&main, "reporting_facts", ("tenant_id", root)).await, total);
        let moved = count(&main, "reporting_facts", ("source_tenant_id", north)).await;
        assert!(moved >= 1, "{moved}");
        assert_eq!(count(&main, "reporting_daily", ("source_tenant_id", north)).await, moved);
        let moved_tenant = tenant(&app, "a-north").await;
        assert_eq!(
            count(moved_tenant.database(), "reporting_rehomes", ("tenant_id", north)).await,
            1
        );
    })
    .await
    .unwrap();
}
