//! Dời khách giữa các placement (bước 33 C2): dry-run không đổi gì; dời thật qua REST (job nền)
//! chép mọi dòng của khách — kể cả job chưa chạy và event chưa giao — sang database riêng, khách
//! nhận 503 trong lúc dời, xong thì dữ liệu ở đích, nguồn sạch, khách khác không bị chạm; dời về;
//! `keep_source` và `purge`; huỷ; chạy tiếp sau khi "chết" giữa chừng; các lỗi từ chối. Chạy trên
//! cả SQLite và Postgres (placement là một database thêm cùng loại).

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{
    Actor, Delivered, Event, EventHandler, EventInfo, Outbox, Subscription, outbox_migration,
};
use autoxide_kernel::i18n::Language;
use autoxide_kernel::job::{
    EnqueueOptions, Job, JobContext, JobHandler, JobQueue, JobRegistration, jobs_migration,
};
use autoxide_kernel::migration::prelude::*;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::tenancy::migration::adopt;
use autoxide_kernel::tenancy::{NewPlacement, PlacementKind, TenantContext, TenantId, TenantMoves};
use autoxide_testkit::{TestApp, TestDatabase, tool};
use sea_orm::sea_query::{Alias, Expr, Query};
use serde::Serialize;
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Serialize)]
struct Ship {
    item: String,
}

impl Job for Ship {
    const KIND: &'static str = "shop.ship";
}

#[derive(Serialize)]
struct Made {
    name: String,
}

impl Event for Made {
    const NAME: &'static str = "shop.made";
}

/// Job `shop.ship` thêm một món `shipped:<item>` vào database của khách; event `shop.made` được
/// ghi lại (khách, tên).
#[derive(Default)]
struct Recorder {
    events: Mutex<Vec<(TenantId, String)>>,
}

#[async_trait::async_trait]
impl JobHandler for Recorder {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let tenant = job.tenant.clone().unwrap();
        let item = job.payload["item"].as_str().unwrap_or_default();
        insert_item(&tenant, &format!("shipped:{item}")).await;
        Ok(())
    }
}

#[async_trait::async_trait]
impl EventHandler for Recorder {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let name = event.payload["name"].as_str().unwrap_or_default().to_owned();
        self.events.lock().unwrap().push((event.tenant.id(), name));
        Ok(())
    }
}

struct CreateItems;

impl MigrationName for CreateItems {
    fn name(&self) -> &str {
        "m0001_create_items"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateItems {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("shop_items")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("id"))
                    .col(schema::string_len("name", 100))
                    .col(schema::big_integer("price_minor"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .primary_key(Index::create().col("tenant_id").col("id"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("shop_items").to_owned()).await
    }
}

struct Shop {
    recorder: Arc<Recorder>,
    outbox: Arc<Mutex<Option<Outbox>>>,
}

impl Module for Shop {
    fn name(&self) -> &'static str {
        "shop"
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(CreateItems),
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
            // Đường người làm vào khách (bước 34, D-310).
            autoxide_kernel::event::outbox_access_migration("k0001_event_access", "shop"),
        ]
    }

    fn events(&self) -> Vec<EventInfo> {
        vec![EventInfo::new("shop.made", "An item was made.")]
    }

    fn jobs(&self) -> Vec<JobRegistration> {
        vec![JobRegistration::new("shop.ship", self.recorder.clone())]
    }

    fn subscriptions(&self) -> Vec<Subscription> {
        vec![Subscription::new("shop.made", self.recorder.clone())]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        *self.outbox.lock().unwrap() = Some(context.outbox()?);
        Ok(())
    }
}

async fn insert_item(tenant: &TenantContext, name: &str) {
    let insert = Query::insert()
        .into_table(Alias::new("shop_items"))
        .columns(["tenant_id", "id", "name", "price_minor", "created_at"].map(Alias::new))
        .values_panic([
            tenant.id().as_uuid().into(),
            Uuid::now_v7().into(),
            name.into(),
            1999i64.into(),
            chrono::Utc::now().into(),
        ])
        .to_owned();
    tenant.database().write().execute(&insert).await.unwrap();
}

/// Số dòng của khách `tenant` trong `table` ở database `database`.
async fn rows(
    database: &autoxide_kernel::database::Database,
    table: &str,
    tenant: TenantId,
) -> i64 {
    let select = Query::select()
        .expr_as(Expr::cust("COUNT(*)"), Alias::new("n"))
        .from(Alias::new(table))
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.as_uuid()))
        .to_owned();
    let row = database.write().query_one(&select).await.unwrap().unwrap();
    row.try_get("", "n").unwrap()
}

const PLATFORM: [&str; 3] = ["kernel.tenants.move", "kernel.tenants.read", "kernel.tenants.manage"];

struct World {
    app: TestApp,
    recorder: Arc<Recorder>,
    outbox: Outbox,
}

async fn world(test: &TestDatabase) -> World {
    let recorder = Arc::new(Recorder::default());
    let slot = Arc::new(Mutex::new(None));
    let app = TestApp::builder(test)
        // Dời chờ TTL cache + thời gian tối đa của request: rút ngắn cho test.
        .setting("AUTOXIDE_TENANCY_CACHE_SECONDS", "1")
        .setting("AUTOXIDE_HTTP_REQUEST_TIMEOUT_SECONDS", "1")
        .module(Shop { recorder: recorder.clone(), outbox: slot.clone() })
        .tenant("a")
        .tenant("b")
        .placement("spare")
        .user("platform", 1, &PLATFORM)
        .user_in("a", "token-a", 2, &[])
        .build()
        .await
        .unwrap();
    let outbox = slot.lock().unwrap().clone().unwrap();
    World { app, recorder, outbox }
}

impl World {
    async fn tenant(&self, slug: &str) -> TenantContext {
        self.app.registry().invalidate(self.app.tenant(slug).await.unwrap().id());
        self.app.tenant(slug).await.unwrap()
    }

    fn moves(&self) -> Arc<TenantMoves> {
        self.app.api::<TenantMoves>().unwrap()
    }

    /// Ba món, một job chưa chạy và một event chưa giao cho `a`; hai món cho `b`.
    async fn seed(&self) {
        let a = self.tenant("a").await;
        for name in ["kettle", "teapot", "cup"] {
            insert_item(&a, name).await;
        }
        let queue = JobQueue::new("shop");
        let ship = Ship { item: "kettle".to_owned() };
        queue.enqueue(a.database().write(), &a, &ship, EnqueueOptions::default()).await.unwrap();
        let made = Made { name: "kettle".to_owned() };
        self.outbox.record(a.database().write(), &a, Actor::system(), None, &made).await.unwrap();
        let b = self.tenant("b").await;
        for name in ["plate", "fork"] {
            insert_item(&b, name).await;
        }
    }

    async fn placement_of(&self, slug: &str) -> String {
        let answer =
            self.app.get(&format!("/api/v1/kernel/tenants/{slug}")).token("platform").send().await;
        let body = answer.unwrap().json();
        body["data"]["placement"].as_str().unwrap().to_owned()
    }
}

#[tokio::test]
async fn dry_run_khong_doi_gi_roi_doi_that_qua_rest_va_doi_ve() {
    TestDatabase::run(async |test| {
        let world = world(test).await;
        world.seed().await;
        let a = world.tenant("a").await;
        let main = a.database().clone();

        // Dry-run (mặc định): kế hoạch, không đổi gì.
        let answer = world
            .app
            .post("/api/v1/kernel/tenants/a/move")
            .token("platform")
            .json(json!({"to": "spare"}))
            .send()
            .await
            .unwrap();
        assert_eq!(answer.status(), 200);
        let plan = answer.json()["data"].clone();
        assert_eq!(plan["ready"], json!(true), "{plan}");
        assert_eq!(plan["from"], json!("main"));
        assert_eq!(plan["to"], json!("spare"));
        let count = |table: &str| {
            plan["tables"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["table"] == json!(table))
                .map(|entry| entry["rows"].as_u64().unwrap())
        };
        assert_eq!(count("shop_items"), Some(3));
        assert_eq!(count("shop_jobs"), Some(1));
        assert_eq!(count("shop_events"), Some(1));
        assert_eq!(count("kernel_tenants"), None, "bảng mặt điều khiển không chép");
        assert_eq!(world.placement_of("a").await, "main");
        assert_eq!(world.tenant("a").await.status().as_str(), "active");

        // Qua MCP: cùng service, dry-run mặc định.
        let client = world.app.mcp("platform").await.unwrap();
        let called = client
            .call_tool(tool("kernel.move_tenant", json!({"tenant": "a", "to": "spare"})))
            .await
            .unwrap();
        let text = serde_json::to_string(&called.structured_content).unwrap();
        assert!(text.contains("shop_items"), "{text}");
        client.cancel().await.unwrap();
        assert_eq!(world.tenant("a").await.status().as_str(), "active");

        // Dời thật: 201, khách nhận 503 trong lúc chờ job.
        let answer = world
            .app
            .post("/api/v1/kernel/tenants/a/move")
            .token("platform")
            .json(json!({"to": "spare", "dry_run": false}))
            .send()
            .await
            .unwrap();
        assert_eq!(answer.status(), 201);
        let location = answer.header("location").to_owned();
        let started = answer.json()["data"].clone();
        assert_eq!(started["state"], json!("copying"));
        let status_path = location.trim_start_matches("/api/v1").to_owned();
        assert!(status_path.ends_with(started["id"].as_str().unwrap()), "{location}");
        let busy = world.app.get("/api/v1/kernel/tenant").token("token-a").send().await.unwrap();
        assert_eq!(busy.status(), 503);
        assert_eq!(busy.json()["code"], json!("tenant_maintenance"));
        assert!(!busy.header("retry-after").is_empty());
        // Đổi khách đang dời: từ chối.
        let tenant_id = a.id();
        let patch = world
            .app
            .patch(&format!("/api/v1/kernel/tenants/{tenant_id}"))
            .token("platform")
            .json(json!({"status": "active"}))
            .send()
            .await
            .unwrap();
        assert_eq!(patch.status(), 409);
        assert_eq!(patch.json()["code"], json!("tenant_moving"));

        // Worker chạy job dời (và trả job của khách đang dời về hàng đợi).
        let worker = world.app.modules().worker(4).unwrap();
        worker.run_once().await.unwrap();
        let answer = world.app.get(&format!("/api/v1{status_path}")).token("platform").send().await;
        let moved = answer.unwrap().json()["data"].clone();
        assert_eq!(moved["state"], json!("done"), "{moved}");
        assert_eq!(moved["error"], Value::Null);
        assert_eq!(world.placement_of("a").await, "spare");

        // Dữ liệu ở đích, nguồn sạch, khách `b` không bị chạm.
        let a = world.tenant("a").await;
        let spare = a.database().clone();
        let b = world.tenant("b").await;
        assert_eq!(rows(&spare, "shop_items", a.id()).await, 3);
        assert_eq!(rows(&spare, "shop_jobs", a.id()).await, 1);
        assert_eq!(rows(&spare, "shop_events", a.id()).await, 1);
        for table in ["shop_items", "shop_jobs", "shop_events", "kernel_events"] {
            assert_eq!(rows(&main, table, a.id()).await, 0, "{table}");
        }
        assert_eq!(rows(&main, "shop_items", b.id()).await, 2);
        assert_eq!(rows(&spare, "shop_items", b.id()).await, 0);
        let ok = world.app.get("/api/v1/kernel/tenant").token("token-a").send().await.unwrap();
        assert_eq!(ok.status(), 200);
        assert_eq!(ok.json()["data"]["status"], json!("active"));

        // Job đã được trả về (không tính lần chạy, hẹn sau 30 giây): cho tới giờ ngay rồi chạy —
        // chạy ở đích.
        let select = Query::select()
            .columns(["attempts", "run_at"].map(Alias::new))
            .from(Alias::new("shop_jobs"))
            .to_owned();
        let job = spare.write().query_one(&select).await.unwrap().unwrap();
        assert_eq!(job.try_get::<i32>("", "attempts").unwrap(), 0);
        let update = Query::update()
            .table(Alias::new("shop_jobs"))
            .value(Alias::new("run_at"), chrono::Utc::now() - chrono::Duration::seconds(1))
            .to_owned();
        spare.write().execute(&update).await.unwrap();
        assert!(worker.run_once().await.unwrap() >= 1);
        assert_eq!(rows(&spare, "shop_items", a.id()).await, 4);
        // Event chưa giao đi theo và được giao ở đích, kèm `kernel.tenant_moved`.
        let dispatcher = world.app.modules().dispatcher().unwrap();
        dispatcher.run_once().await.unwrap();
        assert_eq!(*world.recorder.events.lock().unwrap(), [(a.id(), "kettle".to_owned())]);
        let moved_events = Query::select()
            .expr_as(Expr::cust("COUNT(*)"), Alias::new("n"))
            .from(Alias::new("kernel_events"))
            .and_where(Expr::col(Alias::new("name")).eq("kernel.tenant_moved"))
            .and_where(Expr::col(Alias::new("delivered_at")).is_not_null())
            .to_owned();
        let row = spare.write().query_one(&moved_events).await.unwrap().unwrap();
        assert_eq!(row.try_get::<i64>("", "n").unwrap(), 1);

        // Dời về `main` bằng service (đường của CLI), chạy ngay.
        let moves = world.moves();
        let back = moves.start(Actor::system(), "a", "main", false, false).await.unwrap();
        let back = moves.run(back.id, "test").await.unwrap();
        assert_eq!(back.state, "done");
        assert_eq!(world.placement_of("a").await, "main");
        assert_eq!(rows(&main, "shop_items", a.id()).await, 4);
        assert_eq!(rows(&spare, "shop_items", a.id()).await, 0);
        assert_eq!(rows(&main, "shop_items", b.id()).await, 2);
        let item = back.tables.iter().find(|table| table.table == "shop_items").unwrap();
        assert_eq!(item.rows, 4);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn giu_ban_cu_huy_chay_tiep_va_tu_choi() {
    TestDatabase::run(async |test| {
        let world = world(test).await;
        world.seed().await;
        let moves = world.moves();
        let a = world.tenant("a").await;
        let b = world.tenant("b").await;
        let main = a.database().clone();
        let code = |result: Result<_, Error>| {
            result.map(|_: autoxide_kernel::tenancy::TenantMove| ()).unwrap_err().code().to_owned()
        };

        // Từ chối.
        let start = async |tenant: &str, to: &str| {
            code(moves.start(Actor::system(), tenant, to, false, false).await)
        };
        assert_eq!(start("default", "spare").await, "tenant_move_platform");
        assert_eq!(start("a", "main").await, "tenant_move_same_placement");
        assert_eq!(start("a", "nowhere").await, "tenant_placement_not_found");
        assert_eq!(start("nobody", "spare").await, "tenant_not_found");
        let suspend = world
            .app
            .patch(&format!("/api/v1/kernel/tenants/{}", b.id()))
            .token("platform")
            .json(json!({"status": "suspended"}))
            .send()
            .await
            .unwrap();
        assert_eq!(suspend.status(), 200);
        assert_eq!(start("b", "spare").await, "tenant_move_not_active");
        let resume = world
            .app
            .patch(&format!("/api/v1/kernel/tenants/{}", b.id()))
            .token("platform")
            .json(json!({"status": "active"}))
            .send()
            .await
            .unwrap();
        assert_eq!(resume.status(), 200);
        // Placement chưa migrate, và placement cùng database với `main`: dry-run nêu mọi lý do.
        let placements = world.app.modules().placements(world.app.registry()).unwrap();
        let empty = test.extra_database().await.unwrap();
        placements
            .add(NewPlacement::new("empty", PlacementKind::Dedicated, empty.parse().unwrap()))
            .await
            .unwrap();
        placements
            .add(NewPlacement::new("alias", PlacementKind::Shared, test.url().parse().unwrap()))
            .await
            .unwrap();
        let plan = moves.plan("a", "empty", false, Language::En).await.unwrap();
        let codes: Vec<&str> = plan.blocking.iter().map(|blocker| blocker.code.as_str()).collect();
        assert_eq!(codes, ["tenant_placement_behind", "tenant_move_schema_differs"], "{plan:?}");
        assert!(!plan.ready);
        assert_eq!(start("a", "empty").await, "tenant_placement_behind");
        let plan = moves.plan("a", "alias", false, Language::Vi).await.unwrap();
        assert_eq!(plan.blocking[0].code, "tenant_move_same_database");
        assert!(plan.blocking[0].message.contains("cùng database"), "{plan:?}");

        // Giữ bản cũ: dữ liệu còn ở nguồn, hàng đợi job và outbox ở nguồn thì không.
        let kept = moves.start(Actor::system(), "a", "spare", true, false).await.unwrap();
        let kept = moves.run(kept.id, "test").await.unwrap();
        assert_eq!((kept.state.as_str(), kept.keep_source), ("done", true));
        let spare = world.tenant("a").await.database().clone();
        assert_eq!(rows(&spare, "shop_items", a.id()).await, 3);
        assert_eq!(rows(&main, "shop_items", a.id()).await, 3);
        assert_eq!(rows(&main, "shop_jobs", a.id()).await, 0);
        assert_eq!(rows(&main, "shop_events", a.id()).await, 0);
        assert_eq!(
            code(moves.purge("a", "spare").await.map(|_| kept.clone())),
            "tenant_purge_current"
        );
        let purged = moves.purge("a", "main").await.unwrap();
        assert!(purged.iter().any(|table| table.table == "shop_items" && table.rows == 3));
        assert_eq!(rows(&main, "shop_items", a.id()).await, 0);
        assert_eq!(rows(&main, "shop_items", b.id()).await, 2);

        // Huỷ lượt dời chưa đổi placement: dòng đã chép ở đích bị xoá, khách trở lại `active`.
        let moving = moves.start(Actor::system(), "b", "spare", false, false).await.unwrap();
        assert_eq!(start("b", "empty").await, "tenant_move_unfinished");
        let copied = moves.step(moving.id, "crashed").await.unwrap();
        assert_eq!(copied.state, "copied");
        assert_eq!(rows(&spare, "shop_items", b.id()).await, 2);
        let aborted = moves.abort("b").await.unwrap();
        assert_eq!(aborted.state, "aborted");
        assert_eq!(rows(&spare, "shop_items", b.id()).await, 0);
        assert_eq!(rows(&main, "shop_items", b.id()).await, 2);
        assert_eq!(world.tenant("b").await.status().as_str(), "active");
        assert_eq!(world.placement_of("b").await, "main");

        // Chết sau khi chép, trước khi đổi placement: chạy lại thì tiếp đúng lượt đó, tới xong.
        let moving = moves.start(Actor::system(), "b", "spare", false, false).await.unwrap();
        let copied = moves.step(moving.id, "crashed").await.unwrap();
        assert_eq!(copied.state, "copied");
        let plan = moves.plan("b", "spare", false, Language::En).await.unwrap();
        assert_eq!((plan.ready, plan.resumes), (true, Some(moving.id)));
        let resumed = moves.start(Actor::system(), "b", "spare", false, false).await.unwrap();
        assert_eq!(resumed.id, moving.id);
        let switched = moves.step(moving.id, "crashed-again").await.unwrap();
        assert_eq!(switched.state, "switched");
        // Đã đổi placement: không huỷ được nữa, chỉ chạy tiếp.
        assert_eq!(code(moves.abort("b").await), "tenant_move_not_abortable");
        let done = moves.run(moving.id, "test").await.unwrap();
        assert_eq!(done.state, "done");
        assert_eq!(world.placement_of("b").await, "spare");
        assert_eq!(rows(&spare, "shop_items", b.id()).await, 2);
        assert_eq!(rows(&main, "shop_items", b.id()).await, 0);
        let fetched = moves.get("b", moving.id).await.unwrap();
        assert_eq!(fetched, done);
        assert_eq!(code(moves.get("a", moving.id).await), "tenant_move_not_found");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn chi_nhan_su_nen_tang_doi_duoc_khach() {
    TestDatabase::run(async |test| {
        let world = world(test).await;
        let refused = world
            .app
            .post("/api/v1/kernel/tenants/a/move")
            .token("token-a")
            .json(json!({"to": "spare"}))
            .send()
            .await
            .unwrap();
        assert_eq!(refused.status(), 403);
        let missing = world
            .app
            .post("/api/v1/kernel/tenants/a/move")
            .token("platform")
            .json(json!({"to": "nowhere"}))
            .send()
            .await
            .unwrap();
        assert_eq!(missing.status(), 404);
        assert_eq!(missing.json()["code"], json!("tenant_placement_not_found"));
        let platform = world
            .app
            .post("/api/v1/kernel/tenants/default/move")
            .token("platform")
            .json(json!({"to": "spare", "dry_run": false}))
            .send()
            .await
            .unwrap();
        assert_eq!(platform.status(), 409);
        assert_eq!(platform.json()["code"], json!("tenant_move_platform"));
    })
    .await
    .unwrap();
}
