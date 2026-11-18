//! Tách / gộp khách (bước 38, `tenancy::relocate`) với một module mẫu khai chính sách cho từng bảng:
//! tách nhánh `north` của khách `a` (database chính) thành khách riêng ở database `spare`, giữ khoá
//! chính và đổi `tenant_id`; gộp ngược lại thì mọi dòng của `a` y như trước khi tách. Kế hoạch
//! (dry-run) báo số dòng và xung đột; chạy lại được sau khi "chết" ở mỗi bước; REST chỉ cho nhân sự
//! nền tảng, khách chỉ gửi yêu cầu. Chạy trên cả SQLite và Postgres.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_kernel::database::Database;
use autoxide_kernel::event::{Actor, outbox_migration};
use autoxide_kernel::i18n::Language;
use autoxide_kernel::job::jobs_migration;
use autoxide_kernel::migration::prelude::*;
use autoxide_kernel::module::Module;
use autoxide_kernel::tenancy::migration::adopt;
use autoxide_kernel::tenancy::{
    OrgId, OrgTree, Relocation, RelocationInput, RelocationKind, TablePolicy, TenantContext,
    TenantId, TenantRelocations,
};
use autoxide_testkit::{TestApp, TestDatabase};
use sea_orm::sea_query::{Alias, Expr, Query};
use serde_json::{Value, json};
use uuid::Uuid;

/// Bảng của module mẫu `shop`.
struct CreateShop;

impl MigrationName for CreateShop {
    fn name(&self) -> &str {
        "m0001_shop"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateShop {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("shop_items")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid_null("org_id"))
                    .col(schema::string_len("name", 100))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("shop_items_name_unique")
                    .table("shop_items")
                    .unique()
                    .col("tenant_id")
                    .col("name")
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("shop_lines")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("item_id"))
                    .col(schema::big_integer("quantity"))
                    .foreign_key(
                        ForeignKey::create()
                            .name("shop_lines_item_fk")
                            .from("shop_lines", "item_id")
                            .to("shop_items", "id"),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("shop_notes")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid_null("org_id"))
                    .col(schema::uuid_null("item_id"))
                    .foreign_key(
                        ForeignKey::create()
                            .name("shop_notes_item_fk")
                            .from("shop_notes", "item_id")
                            .to("shop_items", "id"),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("shop_settings")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("key", 50))
                    .col(schema::string_len("value", 100))
                    .primary_key(Index::create().col("tenant_id").col("key"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("shop_buyers")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("email", 100))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in ["shop_buyers", "shop_settings", "shop_notes", "shop_lines", "shop_items"] {
            manager.drop_table(Table::drop().table(table).to_owned()).await?;
        }
        Ok(())
    }
}

struct Shop;

impl Module for Shop {
    fn name(&self) -> &'static str {
        "shop"
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(CreateShop),
            jobs_migration("m0002_create_jobs", "shop"),
            outbox_migration("m0003_create_events", "shop"),
            autoxide_kernel::event::outbox_actor_migration("m0004_add_event_actor", "shop"),
            adopt(
                "m0005_tenancy",
                vec![
                    autoxide_kernel::job::tenant_table("shop"),
                    autoxide_kernel::event::tenant_table("shop"),
                ],
            ),
            autoxide_kernel::event::outbox_access_migration("k0001_event_access", "shop"),
        ]
    }

    fn relocation(&self) -> Relocation {
        Relocation::new()
            .by_org(&["shop_items", "shop_notes"])
            .follow("shop_lines", "item_id", "shop_items")
            .copy_on_split(&["shop_settings"])
            .table(
                "shop_buyers",
                TablePolicy::TenantWide {
                    on_split: autoxide_kernel::tenancy::Split::Stay,
                    on_merge: autoxide_kernel::tenancy::Merge::Import,
                },
            )
    }
}

const PLATFORM: [&str; 2] = ["kernel.tenants.relocate", "kernel.tenants.read"];

async fn app(test: &TestDatabase) -> TestApp {
    TestApp::builder(test)
        // Chờ TTL cache + thời gian tối đa của request: rút ngắn cho test.
        .setting("AUTOXIDE_TENANCY_CACHE_SECONDS", "1")
        .setting("AUTOXIDE_HTTP_REQUEST_TIMEOUT_SECONDS", "1")
        .setting("AUTOXIDE_RELOCATION_DRAIN_SECONDS", "5")
        .module(Shop)
        .tenant("a")
        .tenant("b")
        .placement("spare")
        .user("platform", 1, &PLATFORM)
        .user_in("a", "token-a", 2, &["kernel.relocations.request"])
        .user_in("b", "token-b", 3, &["kernel.relocations.request"])
        .build()
        .await
        .unwrap()
}

async fn tenant(app: &TestApp, slug: &str) -> TenantContext {
    let id = app.tenant(slug).await.unwrap().id();
    app.registry().invalidate(id);
    app.tenant(slug).await.unwrap()
}

async fn insert(database: &Database, table: &str, columns: &[&str], values: Vec<sea_orm::Value>) {
    let insert = Query::insert()
        .into_table(Alias::new(table))
        .columns(columns.iter().map(|column| Alias::new(*column)))
        .values_panic(values.into_iter().map(Into::into))
        .to_owned();
    database.write().execute(&insert).await.unwrap();
}

/// Cây của `a`: gốc → north → north-shop; gốc → south. Món, dòng, ghi chú, cài đặt, người mua.
struct Seeded {
    north: OrgId,
    north_shop: OrgId,
    south: OrgId,
}

async fn seed(app: &TestApp) -> Seeded {
    // Event của lúc tạo khách: giao hết (tách / gộp chờ hàng đợi của khách rỗng).
    let dispatcher = app.modules().dispatcher().unwrap();
    while dispatcher.run_once().await.unwrap() > 0 {}
    let a = tenant(app, "a").await;
    let (north, north_shop, south) = (
        OrgId::from_uuid(Uuid::now_v7()),
        OrgId::from_uuid(Uuid::now_v7()),
        OrgId::from_uuid(Uuid::now_v7()),
    );
    let write = a.database().begin_write().await.unwrap();
    let root = OrgTree::insert_root(&write, a.id()).await.unwrap();
    OrgTree::insert(&write, a.id(), north, root, 8).await.unwrap();
    OrgTree::insert(&write, a.id(), north_shop, north, 8).await.unwrap();
    OrgTree::insert(&write, a.id(), south, root, 8).await.unwrap();
    write.commit().await.unwrap();
    let db = a.database();
    let t = a.id().as_uuid();
    for (name, org) in [("kettle", north_shop), ("teapot", north), ("cup", south)] {
        let item = Uuid::now_v7();
        insert(
            db,
            "shop_items",
            &["id", "tenant_id", "org_id", "name"],
            vec![item.into(), t.into(), org.as_uuid().into(), name.into()],
        )
        .await;
        insert(
            db,
            "shop_lines",
            &["id", "tenant_id", "item_id", "quantity"],
            vec![Uuid::now_v7().into(), t.into(), item.into(), 2i64.into()],
        )
        .await;
    }
    insert(
        db,
        "shop_settings",
        &["tenant_id", "key", "value"],
        vec![t.into(), "currency".into(), "VND".into()],
    )
    .await;
    insert(
        db,
        "shop_buyers",
        &["id", "tenant_id", "email"],
        vec![Uuid::now_v7().into(), t.into(), "an@example.com".into()],
    )
    .await;
    Seeded { north, north_shop, south }
}

/// Mọi dòng của khách `tenant` trong các bảng của `shop` và cây tổ chức, dạng chữ, theo thứ tự.
/// Cột `u:` là UUID, `n:` số nguyên 4 byte, `i:` số nguyên 8 byte, còn lại chữ.
async fn dump(database: &Database, tenant: TenantId) -> Vec<String> {
    let tables: [(&str, &[&str]); 7] = [
        ("shop_items", &["u:id", "u:org_id", "name"]),
        ("shop_lines", &["u:id", "u:item_id", "i:quantity"]),
        ("shop_notes", &["u:id", "u:org_id", "u:item_id"]),
        ("shop_settings", &["key", "value"]),
        ("shop_buyers", &["u:id", "email"]),
        ("kernel_org_units", &["u:id", "u:parent_id", "n:depth"]),
        ("kernel_org_closure", &["u:ancestor_id", "u:descendant_id", "n:depth"]),
    ];
    let mut rows = Vec::new();
    for (table, columns) in tables {
        let names: Vec<&str> =
            columns.iter().map(|column| column.rsplit(':').next().unwrap()).collect();
        let select = Query::select()
            .columns(names.iter().map(|name| Alias::new(*name)))
            .from(Alias::new(table))
            .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.as_uuid()))
            .to_owned();
        for row in database.write().query_all(&select).await.unwrap() {
            let values: Vec<String> = columns
                .iter()
                .zip(&names)
                .map(|(column, name)| {
                    if column.starts_with("u:") {
                        row.try_get::<Option<Uuid>>("", name).unwrap().map(|id| id.to_string())
                    } else if column.starts_with("n:") {
                        row.try_get::<Option<i32>>("", name).unwrap().map(|n| n.to_string())
                    } else if column.starts_with("i:") {
                        row.try_get::<Option<i64>>("", name).unwrap().map(|n| n.to_string())
                    } else {
                        row.try_get::<Option<String>>("", name).unwrap()
                    }
                    .unwrap_or_default()
                })
                .collect();
            rows.push(format!("{table}:{}", values.join("|")));
        }
    }
    rows.sort();
    rows
}

async fn count(database: &Database, table: &str, tenant: TenantId) -> i64 {
    let select = Query::select()
        .expr_as(Expr::cust("COUNT(*)"), Alias::new("n"))
        .from(Alias::new(table))
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.as_uuid()))
        .to_owned();
    database.write().query_one(&select).await.unwrap().unwrap().try_get("", "n").unwrap()
}

fn split(north: OrgId) -> RelocationInput {
    RelocationInput {
        kind: RelocationKind::Split,
        tenant: "a".to_owned(),
        org_id: Some(north.as_uuid()),
        into: None,
        under_org: None,
        slug: Some("a-north".to_owned()),
        name: Some("A North".to_owned()),
        placement: Some("spare".to_owned()),
        keep_parent: true,
        reason: "The north branch becomes its own company (contract 2026-11).".to_owned(),
    }
}

fn merge(root: OrgId) -> RelocationInput {
    RelocationInput {
        kind: RelocationKind::Merge,
        tenant: "a-north".to_owned(),
        org_id: None,
        into: Some("a".to_owned()),
        under_org: Some(root.as_uuid()),
        slug: None,
        name: None,
        placement: None,
        keep_parent: true,
        reason: "The north branch comes back.".to_owned(),
    }
}

fn table<'a>(plan: &'a Value, name: &str) -> &'a Value {
    plan["tables"].as_array().unwrap().iter().find(|table| table["table"] == json!(name)).unwrap()
}

#[tokio::test]
async fn split_then_merge_roundtrip_restores_original_data() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let seeded = seed(&app).await;
        let service = app.api::<TenantRelocations>().unwrap();
        let a = tenant(&app, "a").await;
        let main = a.database().clone();
        let before = dump(&main, a.id()).await;

        // Dry-run: số dòng mỗi bảng, không đổi gì.
        let plan = service.plan(&split(seeded.north), Language::En).await.unwrap();
        let plan = serde_json::to_value(plan).unwrap();
        assert_eq!(plan["ready"], json!(true), "{plan}");
        assert_eq!(table(&plan, "shop_items")["rows"], json!(2));
        assert_eq!(table(&plan, "shop_items")["action"], json!("move"));
        assert_eq!(table(&plan, "shop_lines")["rows"], json!(2));
        assert_eq!(table(&plan, "shop_settings")["action"], json!("copy"));
        assert_eq!(table(&plan, "shop_buyers")["action"], json!("stay"));
        assert_eq!(table(&plan, "kernel_org_units")["rows"], json!(2));
        assert_eq!(dump(&main, a.id()).await, before);

        // Tách: khách mới có ID của nút `north`, ở database `spare`.
        let started =
            service.start(Actor::system(), &split(seeded.north), None, false).await.unwrap();
        let done = service.run(started.id, "test").await.unwrap();
        assert_eq!(done.state, "done", "{done:?}");
        let north = tenant(&app, "a-north").await;
        assert_eq!(north.id().as_uuid(), seeded.north.as_uuid());
        assert_eq!(north.status().as_str(), "active");
        let spare = north.database().clone();
        assert_eq!(count(&spare, "shop_items", north.id()).await, 2);
        assert_eq!(count(&spare, "shop_lines", north.id()).await, 2);
        assert_eq!(count(&spare, "shop_settings", north.id()).await, 1);
        assert_eq!(count(&spare, "shop_buyers", north.id()).await, 0);
        assert_eq!(count(&main, "shop_items", a.id()).await, 1);
        assert_eq!(count(&main, "shop_settings", a.id()).await, 1);
        // Nút `north` là gốc của khách mới (độ sâu 0, không cha); `north-shop` ở độ sâu 1.
        let units = dump(&spare, north.id()).await;
        assert!(units.contains(&format!("kernel_org_units:{}||0", seeded.north)), "{units:?}");
        assert!(
            units.contains(&format!("kernel_org_units:{}|{}|1", seeded.north_shop, seeded.north)),
            "{units:?}"
        );
        assert_eq!(count(&spare, "kernel_org_closure", north.id()).await, 3);
        assert_eq!(count(&main, "kernel_org_closure", a.id()).await, 3);
        assert_eq!(tenant(&app, "a").await.status().as_str(), "active");

        // Event `kernel.relocation_finished` ở khách mới: giao trước (gộp chờ hàng đợi rỗng).
        let dispatcher = app.modules().dispatcher().unwrap();
        while dispatcher.run_once().await.unwrap() > 0 {}
        // Gộp ngược lại dưới gốc của `a`: mọi dòng của `a` như trước khi tách.
        let root = OrgId::from_uuid(a.id().as_uuid());
        let plan = service.plan(&merge(root), Language::En).await.unwrap();
        let plan = serde_json::to_value(plan).unwrap();
        assert_eq!(plan["ready"], json!(true), "{plan}");
        // Bản cài đặt đã chép khi tách: cùng khoá chính ở khách nhận, bỏ qua.
        assert_eq!(table(&plan, "shop_settings")["existing"], json!(1));
        let started = service.start(Actor::system(), &merge(root), None, false).await.unwrap();
        let done = service.run(started.id, "test").await.unwrap();
        assert_eq!(done.state, "done", "{done:?}");
        assert_eq!(dump(&main, a.id()).await, before);
        assert_eq!(count(&spare, "shop_items", north.id()).await, 0);
        let gone = app.tenant("a-north").await;
        assert!(gone.is_err() || gone.unwrap().status().as_str() == "merged");
        let _ = seeded.south;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn split_dry_run_reports_conflicts_and_merge_unique_conflict_fails() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let seeded = seed(&app).await;
        let service = app.api::<TenantRelocations>().unwrap();
        let a = tenant(&app, "a").await;
        let main = a.database().clone();
        // Ghi chú ở nút `north` trỏ món của `south`: dòng đi trỏ dòng ở lại.
        let cup: Uuid = {
            let select = Query::select()
                .column(Alias::new("id"))
                .from(Alias::new("shop_items"))
                .and_where(Expr::col(Alias::new("name")).eq("cup"))
                .to_owned();
            main.write().query_one(&select).await.unwrap().unwrap().try_get("", "id").unwrap()
        };
        let note = Uuid::now_v7();
        insert(
            &main,
            "shop_notes",
            &["id", "tenant_id", "org_id", "item_id"],
            vec![note.into(), a.id().as_uuid().into(), seeded.north.as_uuid().into(), cup.into()],
        )
        .await;
        let plan =
            serde_json::to_value(service.plan(&split(seeded.north), Language::En).await.unwrap())
                .unwrap();
        assert_eq!(plan["ready"], json!(false));
        let conflict = &plan["conflicts"][0];
        assert_eq!(conflict["table"], json!("shop_notes"), "{plan}");
        assert_eq!(conflict["code"], json!("reference"));
        assert_eq!(conflict["rows"], json!([note.to_string()]));
        // Chạy thật: lượt `failed` kèm báo cáo, khách vẫn phục vụ.
        let refused = service.start(Actor::system(), &split(seeded.north), None, false).await;
        assert_eq!(refused.unwrap_err().code(), "tenancy.relocation_conflict");
        assert_eq!(tenant(&app, "a").await.status().as_str(), "active");
        let delete = Query::delete().from_table(Alias::new("shop_notes")).to_owned();
        main.write().execute(&delete).await.unwrap();

        // Tách xong, `a` tạo món trùng tên với món đã đi: gộp lại bị chặn, liệt kê dòng.
        let started =
            service.start(Actor::system(), &split(seeded.north), None, false).await.unwrap();
        assert_eq!(service.run(started.id, "test").await.unwrap().state, "done");
        insert(
            &main,
            "shop_items",
            &["id", "tenant_id", "org_id", "name"],
            vec![
                Uuid::now_v7().into(),
                a.id().as_uuid().into(),
                seeded.south.as_uuid().into(),
                "kettle".into(),
            ],
        )
        .await;
        let root = OrgId::from_uuid(a.id().as_uuid());
        let plan =
            serde_json::to_value(service.plan(&merge(root), Language::En).await.unwrap()).unwrap();
        let unique: Vec<&Value> = plan["conflicts"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|conflict| conflict["code"] == json!("unique"))
            .collect();
        assert_eq!(unique.len(), 1, "{plan}");
        assert_eq!(unique[0]["table"], json!("shop_items"));
        assert_eq!(unique[0]["count"], json!(1));
        let refused = service.start(Actor::system(), &merge(root), None, false).await;
        assert_eq!(refused.unwrap_err().code(), "tenancy.relocation_conflict");
        // Khách không phải nút trong cây, cùng database, khách không có: lý do chặn rõ.
        let mut wrong = split(OrgId::from_uuid(Uuid::now_v7()));
        wrong.slug = Some("a-x".to_owned());
        let plan = serde_json::to_value(service.plan(&wrong, Language::En).await.unwrap()).unwrap();
        let codes: Vec<&str> = plan["blocking"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["code"].as_str().unwrap())
            .collect();
        assert!(codes.contains(&"tenancy.relocation_org_invalid"), "{codes:?}");
        let mut same = split(seeded.south);
        same.slug = Some("a-south".to_owned());
        same.placement = Some("main".to_owned());
        let plan = serde_json::to_value(service.plan(&same, Language::En).await.unwrap()).unwrap();
        let codes: Vec<&str> = plan["blocking"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["code"].as_str().unwrap())
            .collect();
        assert!(codes.contains(&"tenancy.relocation_same_database"), "{codes:?}");
        let mut empty = split(seeded.south);
        empty.reason = " ".to_owned();
        assert_eq!(
            service.plan(&empty, Language::En).await.unwrap_err().code(),
            "validation_failed"
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn split_resume_after_failure_in_each_state() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let seeded = seed(&app).await;
        let service = app.api::<TenantRelocations>().unwrap();
        let started =
            service.start(Actor::system(), &split(seeded.north), None, false).await.unwrap();
        // Mỗi bước một bên chạy khác ("chết" sau mỗi bước, lease đã trả): bên mới chạy tiếp.
        let mut states = Vec::new();
        for attempt in 0..10 {
            let step = service.step(started.id, &format!("runner-{attempt}")).await.unwrap();
            states.push(step.state.clone());
            if step.state == "done" {
                break;
            }
            // Trong lúc khoá: chỉ khách `a` trả 503, khách `b` vẫn chạy.
            if step.state == "frozen" {
                let a = app.get("/api/v1/kernel/tenant").token("token-a").send().await.unwrap();
                assert_eq!(a.status(), 503);
                assert_eq!(a.json()["code"], json!("tenant_maintenance"));
                let b = app.get("/api/v1/kernel/tenant").token("token-b").send().await.unwrap();
                assert_eq!(b.status(), 200, "{}", b.text());
            }
            // Bước chép chạy lại hai lần: lần sau xoá bản của lần trước rồi chép lại.
            if step.state == "copied" && attempt < 4 {
                let row = service.get(started.id).await.unwrap();
                assert_eq!(row.tables.iter().find(|t| t.table == "shop_items").unwrap().rows, 2);
            }
        }
        assert_eq!(
            states,
            ["frozen", "copied", "verified", "switched", "cleaned", "done"],
            "{states:?}"
        );
        let north = tenant(&app, "a-north").await;
        assert_eq!(count(north.database(), "shop_items", north.id()).await, 2);
        // Lượt đã xong: chạy lại không làm gì.
        assert_eq!(service.run(started.id, "again").await.unwrap().state, "done");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn relocation_api_chi_nhan_su_nen_tang_mac_dinh_dry_run_khach_chi_gui_yeu_cau() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let seeded = seed(&app).await;
        let body = json!({
            "kind": "split",
            "tenant": "a",
            "org_id": seeded.north.as_uuid(),
            "slug": "a-north",
            "name": "A North",
            "placement": "spare",
            "reason": "contract",
        });
        // Người của khách (không phải nhân sự nền tảng): bị từ chối.
        let refused =
            app.post("/api/v1/kernel/relocations").token("token-a").json(body.clone()).send().await.unwrap();
        assert_eq!(refused.status(), 403);
        // Khách chỉ gửi yêu cầu: không đổi gì, vào hộp việc của nhân sự nền tảng.
        let ask = json!({"kind": "split", "org_id": seeded.north.as_uuid(), "slug": "a-north", "name": "A North", "reason": "contract"});
        let asked = app
            .post("/api/v1/kernel/tenant/relocation-requests")
            .token("token-a")
            .header("autoxide-tenant", "a")
            .json(ask)
            .send()
            .await
            .unwrap();
        assert_eq!(asked.status(), 201, "{}", asked.text());
        let request = asked.json()["data"].clone();
        assert_eq!(request["state"], json!("requested"));
        let request_id = request["id"].as_str().unwrap().to_owned();
        let other = app
            .get(&format!("/api/v1/kernel/tenant/relocation-requests/{request_id}"))
            .token("token-b")
            .header("autoxide-tenant", "b")
            .send()
            .await
            .unwrap();
        assert_eq!(other.status(), 404);
        let work = app.get("/api/v1/kernel/work").token("platform").send().await.unwrap();
        assert_eq!(work.status(), 200, "{}", work.text());
        assert!(work.text().contains(&request_id), "{}", work.text());

        // Nhân sự nền tảng: dry-run mặc định, không đổi gì.
        let planned =
            app.post("/api/v1/kernel/relocations").token("platform").json(body.clone()).send().await.unwrap();
        assert_eq!(planned.status(), 200, "{}", planned.text());
        assert_eq!(planned.json()["data"]["ready"], json!(true));
        assert_eq!(tenant(&app, "a").await.status().as_str(), "active");
        // Chạy thật theo yêu cầu: job nền làm.
        let mut run = body.clone();
        run["dry_run"] = json!(false);
        run["request_id"] = json!(request_id);
        let started = app.post("/api/v1/kernel/relocations").token("platform").json(run).send().await.unwrap();
        assert_eq!(started.status(), 201, "{}", started.text());
        let location = started.header("location").to_owned();
        assert!(location.ends_with(&request_id), "{location}");
        app.modules().worker(4).unwrap().run_once().await.unwrap();
        let path = format!("/api/v1/kernel/relocations/{request_id}");
        let status = app.get(&path).token("platform").send().await.unwrap();
        assert_eq!(status.json()["data"]["state"], json!("done"), "{}", status.text());
        let mine = app
            .get(&format!("/api/v1/kernel/tenant/relocation-requests/{request_id}"))
            .token("token-a")
            .header("autoxide-tenant", "a")
            .send()
            .await
            .unwrap();
        assert_eq!(mine.json()["data"]["state"], json!("done"));
        let north = tenant(&app, "a-north").await;
        assert_eq!(count(north.database(), "shop_items", north.id()).await, 2);
    })
    .await
    .unwrap();
}
