//! Job và event theo khách, trên nhiều database (bước 33 A4): worker chia lượt công bằng giữa các
//! khách, khoá chống trùng theo khách, lịch chạy mỗi database một lần, bộ phát giao event trên mọi
//! database kèm đúng khách. Chạy trên cả SQLite và Postgres; placement riêng là một database thêm
//! cùng loại (`TestDatabase::extra_database`).

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use autoxide_kernel::config::{Config, ConfigReader, ConfigSource, DatabaseConfig};
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{
    Actor, Delivered, Event, EventHandler, EventInfo, Outbox, Subscription, outbox_migration,
};
use autoxide_kernel::job::{
    EnqueueOptions, Job, JobContext, JobHandler, JobQueue, JobRegistration, Schedule,
    jobs_migration,
};
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, Modules, StartContext};
use autoxide_kernel::tenancy::migration::adopt;
use autoxide_kernel::tenancy::{
    DEFAULT_TENANT, DatabaseRegistry, MAIN_PLACEMENT, TenancyConfig, TenancyMode, TenantContext,
    TenantId,
};
use autoxide_testkit::TestDatabase;
use sea_orm::ConnectionTrait as _;
use sea_orm::sea_query::{Alias, ExprTrait as _, Query};
use serde::Serialize;
use uuid::Uuid;

#[derive(Serialize)]
struct Send {
    to: String,
}

impl Job for Send {
    const KIND: &'static str = "reports.send";
}

#[derive(Serialize)]
struct Made {
    title: String,
}

impl Event for Made {
    const NAME: &'static str = "reports.made";
}

/// Một lần chạy job: khách (None = job của cả database) và database (URL) đã nhận.
type Run = (Option<TenantId>, String, String);

#[derive(Default)]
struct Recorder {
    runs: Mutex<Vec<Run>>,
    events: Mutex<Vec<(TenantId, String)>>,
}

#[async_trait::async_trait]
impl JobHandler for Recorder {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let to = job.payload["to"].as_str().unwrap_or("schedule").to_owned();
        let tenant = job.tenant.as_ref().map(TenantContext::id);
        let database = database_name(&job.database).await;
        self.runs.lock().unwrap().push((tenant, database, to));
        Ok(())
    }
}

#[async_trait::async_trait]
impl EventHandler for Recorder {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let title = event.payload["title"].as_str().unwrap_or_default().to_owned();
        self.events.lock().unwrap().push((event.tenant.id(), title));
        Ok(())
    }
}

/// Database này là database nào: đọc dấu đặt sẵn trong bảng `reports_mark`.
async fn database_name(database: &Database) -> String {
    let select =
        Query::select().column(Alias::new("name")).from(Alias::new("reports_mark")).to_owned();
    let row = database.read().query_one(&select).await.unwrap().unwrap();
    row.try_get("", "name").unwrap()
}

struct Reports {
    recorder: Arc<Recorder>,
    outbox: Arc<Mutex<Option<Outbox>>>,
    schedules: Vec<Schedule>,
}

struct CreateMark;

impl autoxide_kernel::migration::prelude::MigrationName for CreateMark {
    fn name(&self) -> &str {
        "m0003_create_mark"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateMark {
    async fn up(
        &self,
        manager: &autoxide_kernel::migration::prelude::SchemaManager,
    ) -> Result<(), autoxide_kernel::migration::prelude::DbErr> {
        manager
            .get_connection()
            .execute_unprepared("CREATE TABLE reports_mark (name TEXT)")
            .await?;
        Ok(())
    }

    async fn down(
        &self,
        manager: &autoxide_kernel::migration::prelude::SchemaManager,
    ) -> Result<(), autoxide_kernel::migration::prelude::DbErr> {
        manager.get_connection().execute_unprepared("DROP TABLE reports_mark").await?;
        Ok(())
    }
}

impl Module for Reports {
    fn name(&self) -> &'static str {
        "reports"
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![
            jobs_migration("m0001_create_jobs", "reports"),
            outbox_migration("m0002_create_events", "reports"),
            Box::new(CreateMark),
            autoxide_kernel::event::outbox_actor_migration("m0004_add_event_actor", "reports"),
            adopt(
                "m0005_tenancy",
                vec![
                    autoxide_kernel::job::tenant_table("reports"),
                    autoxide_kernel::event::tenant_table("reports"),
                ],
            ),
        ]
    }

    fn events(&self) -> Vec<EventInfo> {
        vec![EventInfo::new("reports.made", "A report was made.")]
    }

    fn jobs(&self) -> Vec<JobRegistration> {
        vec![JobRegistration::new("reports.send", self.recorder.clone())]
    }

    fn schedules(&self) -> Vec<Schedule> {
        self.schedules.clone()
    }

    fn subscriptions(&self) -> Vec<Subscription> {
        vec![Subscription::new("reports.made", self.recorder.clone())]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        *self.outbox.lock().unwrap() = Some(context.outbox()?);
        Ok(())
    }
}

struct Setup {
    control: Database,
    registry: DatabaseRegistry,
    modules: Modules,
    recorder: Arc<Recorder>,
    outbox: Outbox,
}

fn modules(
    recorder: &Arc<Recorder>,
    schedules: Vec<Schedule>,
) -> (Modules, Arc<Mutex<Option<Outbox>>>) {
    let slot = Arc::new(Mutex::new(None));
    let module = Reports { recorder: recorder.clone(), outbox: slot.clone(), schedules };
    (Modules::new(vec![Box::new(module)]).unwrap(), slot)
}

async fn connect(url: &str) -> (Database, DatabaseConfig) {
    let source = ConfigSource::from_pairs([("DATABASE_URL", url)]);
    let mut reader = ConfigReader::new(&source);
    let config = Config::read(&mut reader);
    let (config, _) = reader.finish(config).unwrap();
    (Database::connect(&config.database).await.unwrap(), config.database)
}

/// Migrate `database` và đặt dấu tên `name` cho nó.
async fn prepare(modules: &Modules, database: &Database, name: &str) {
    modules.migrator().unwrap().up(database).await.unwrap();
    let insert = Query::insert()
        .into_table(Alias::new("reports_mark"))
        .columns([Alias::new("name")])
        .values_panic([name.into()])
        .to_owned();
    database.write().execute(&insert).await.unwrap();
}

/// Database điều khiển + (nếu `placement`) một placement riêng; chế độ `multi`.
async fn setup(test: &TestDatabase, schedules: Vec<Schedule>, placement: bool) -> Setup {
    let recorder = Arc::new(Recorder::default());
    let (mut modules, slot) = modules(&recorder, schedules);
    let (control, base) = connect(test.url()).await;
    prepare(&modules, &control, "main").await;
    if placement {
        let url = test.extra_database().await.unwrap();
        let (other, _) = connect(&url).await;
        prepare(&modules, &other, "p1").await;
        let now = chrono::Utc::now();
        let insert = Query::insert()
            .into_table(Alias::new("kernel_placements"))
            .columns(
                [
                    "id",
                    "name",
                    "kind",
                    "backend",
                    "url_sealed",
                    "schema_state",
                    "created_at",
                    "updated_at",
                ]
                .map(Alias::new),
            )
            .values_panic([
                Uuid::now_v7().into(),
                "p1".into(),
                "dedicated".into(),
                format!("{:?}", test.backend()).to_lowercase().into(),
                url.into(),
                "ready".into(),
                now.into(),
                now.into(),
            ])
            .to_owned();
        control.write().execute(&insert).await.unwrap();
    }
    let registry =
        DatabaseRegistry::new(control.clone(), base, TenancyConfig::with_mode(TenancyMode::Multi));
    modules.start_in(&registry).unwrap();
    let outbox = slot.lock().unwrap().clone().unwrap();
    Setup { control, registry, modules, recorder, outbox }
}

/// Khách mới trên placement `placement` (tên), trạng thái active.
async fn tenant(setup: &Setup, slug: &str, placement: Option<&str>) -> TenantContext {
    let placement = match placement {
        None => MAIN_PLACEMENT.as_uuid(),
        Some(name) => {
            let select = Query::select()
                .column(Alias::new("id"))
                .from(Alias::new("kernel_placements"))
                .and_where(sea_orm::sea_query::Expr::col(Alias::new("name")).eq(name))
                .to_owned();
            let row = setup.control.read().query_one(&select).await.unwrap().unwrap();
            row.try_get::<Uuid>("", "id").unwrap()
        }
    };
    let id = Uuid::now_v7();
    let now = chrono::Utc::now();
    let insert = Query::insert()
        .into_table(Alias::new("kernel_tenants"))
        .columns(
            ["id", "slug", "name", "status", "placement_id", "created_at", "updated_at"]
                .map(Alias::new),
        )
        .values_panic([
            id.into(),
            slug.into(),
            slug.into(),
            "active".into(),
            placement.into(),
            now.into(),
            now.into(),
        ])
        .to_owned();
    setup.control.write().execute(&insert).await.unwrap();
    setup.registry.tenant(TenantId::from_uuid(id)).await.unwrap()
}

/// Job lịch của module nội bộ `kernel` ở chế độ `multi`: `kernel.verify_domains`, chỉ trên database
/// chính (D-190).
const KERNEL_JOBS: usize = 1;

#[tokio::test]
async fn worker_chia_luot_cong_bang() {
    TestDatabase::run(async |test| {
        let setup = setup(test, Vec::new(), false).await;
        let a = tenant(&setup, "a", None).await;
        let b = tenant(&setup, "b", None).await;
        let queue = JobQueue::new("reports");
        // Khách A dồn 1000 job; khách B một job, đưa vào sau cùng.
        let transaction = setup.control.begin_write().await.unwrap();
        for index in 0..1000 {
            let job = Send { to: format!("a-{index}") };
            queue.enqueue(&transaction, &a, &job, EnqueueOptions::default()).await.unwrap();
        }
        transaction.commit().await.unwrap();
        let job = Send { to: "b".to_owned() };
        queue.enqueue(setup.control.write(), &b, &job, EnqueueOptions::default()).await.unwrap();

        let worker = setup.modules.worker(4).unwrap();
        // 4 job của `reports` và job lịch `kernel.verify_domains` (chế độ `multi`, database chính).
        assert_eq!(worker.run_once().await.unwrap(), 4 + KERNEL_JOBS);
        let runs = setup.recorder.runs.lock().unwrap().clone();
        assert!(
            runs.iter().any(|(tenant, _, to)| *tenant == Some(b.id()) && to == "b"),
            "the job of B ran in the first round: {runs:?}"
        );
        assert_eq!(runs.iter().filter(|(tenant, _, _)| *tenant == Some(a.id())).count(), 3);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn job_trung_unique_key_khac_khach_deu_chay() {
    TestDatabase::run(async |test| {
        let setup = setup(test, Vec::new(), false).await;
        let a = tenant(&setup, "a", None).await;
        let b = tenant(&setup, "b", None).await;
        let queue = JobQueue::new("reports");
        let unique = || EnqueueOptions::default().unique_key("daily");
        let write = setup.control.write();
        let job = |to: &str| Send { to: to.to_owned() };
        assert!(queue.enqueue(write, &a, &job("a"), unique()).await.unwrap().is_some());
        assert!(queue.enqueue(write, &b, &job("b"), unique()).await.unwrap().is_some());
        assert!(queue.enqueue(write, &a, &job("again"), unique()).await.unwrap().is_none());

        let worker = setup.modules.worker(4).unwrap();
        assert_eq!(worker.run_once().await.unwrap(), 2 + KERNEL_JOBS);
        let mut seen: Vec<_> = setup
            .recorder
            .runs
            .lock()
            .unwrap()
            .iter()
            .map(|(tenant, _, to)| (*tenant, to.clone()))
            .collect();
        seen.sort();
        let mut expected = vec![(Some(a.id()), "a".to_owned()), (Some(b.id()), "b".to_owned())];
        expected.sort();
        assert_eq!(seen, expected);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn lich_chay_mot_lan_moi_database() {
    TestDatabase::run(async |test| {
        let schedule = Schedule::new("reports.nightly", "reports.send", Duration::from_secs(3600));
        let setup = setup(test, vec![schedule], true).await;
        let worker = setup.modules.worker(4).unwrap();
        // Lịch `reports` một job mỗi database; lịch của kernel chỉ ở database chính.
        assert_eq!(worker.run_once().await.unwrap(), 2 + KERNEL_JOBS);
        // Lượt sau cùng khoảng: không thêm job nào; worker khác cũng không.
        assert_eq!(worker.run_once().await.unwrap(), 0);
        assert_eq!(setup.modules.worker(4).unwrap().run_once().await.unwrap(), 0);
        let mut runs = setup.recorder.runs.lock().unwrap().clone();
        runs.sort();
        assert_eq!(
            runs,
            [
                (None, "main".to_owned(), "schedule".to_owned()),
                (None, "p1".to_owned(), "schedule".to_owned())
            ]
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dispatcher_giao_event_tren_moi_database() {
    TestDatabase::run(async |test| {
        let setup = setup(test, Vec::new(), true).await;
        let platform = setup.registry.tenant(DEFAULT_TENANT).await.unwrap();
        let far = tenant(&setup, "far", Some("p1")).await;
        assert_eq!(database_name(far.database()).await, "p1");
        for (tenant, title) in [(&platform, "on main"), (&far, "on p1")] {
            let transaction = tenant.database().begin_write().await.unwrap();
            let event = Made { title: title.to_owned() };
            setup.outbox.record(&transaction, tenant, Actor::system(), None, &event).await.unwrap();
            transaction.commit().await.unwrap();
        }
        // Job của khách trên placement riêng chạy trên database của placement đó.
        let job = Send { to: "far".to_owned() };
        let queue = JobQueue::new("reports");
        queue.enqueue(far.database().write(), &far, &job, EnqueueOptions::default()).await.unwrap();

        let dispatcher = setup.modules.dispatcher().unwrap();
        assert_eq!(dispatcher.run_once().await.unwrap(), 2);
        assert_eq!(dispatcher.run_once().await.unwrap(), 0);
        let mut events = setup.recorder.events.lock().unwrap().clone();
        events.sort();
        let mut expected =
            vec![(DEFAULT_TENANT, "on main".to_owned()), (far.id(), "on p1".to_owned())];
        expected.sort();
        assert_eq!(events, expected);

        assert_eq!(setup.modules.worker(4).unwrap().run_once().await.unwrap(), 1 + KERNEL_JOBS);
        let runs = setup.recorder.runs.lock().unwrap().clone();
        assert_eq!(runs, [(Some(far.id()), "p1".to_owned(), "far".to_owned())]);
    })
    .await
    .unwrap();
}
