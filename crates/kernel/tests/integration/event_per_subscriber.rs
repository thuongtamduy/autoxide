//! Giao event theo từng bên nhận (bước 71 G1, D-20261128-event-per-subscriber), trên database gate đang chọn
//! (SQLite hoặc Postgres): một module lỗi không chặn module đăng ký sau nó; chỉ bên lỗi thử lại, có dead-letter
//! riêng; nhiều bộ phát cùng lúc không giao trùng; event đang chờ từ bản cũ vẫn được giao sau khi nâng cấp.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use autoxide_kernel::config::{ConfigReader, ConfigSource, DatabaseConfig};
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{
    Actor, Delivered, Event, EventHandler, EventInfo, MAX_ATTEMPTS, Outbox, Subscription,
    outbox_migration,
};
use autoxide_kernel::job::backlog::backlog;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, Modules, StartContext};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_testkit::TestDatabase;
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Query};
use sea_orm::{ConnectionTrait, Statement};
use serde::Serialize;
use uuid::Uuid;

const NOTE_CREATED: EventInfo = EventInfo::new("notes.note_created", "A note was created.");

#[derive(Serialize)]
struct NoteCreated {
    title: String,
}

impl Event for NoteCreated {
    const NAME: &'static str = "notes.note_created";
}

/// Module phát event.
struct Notes(Arc<Mutex<Option<Outbox>>>);

impl Module for Notes {
    fn name(&self) -> &'static str {
        "notes"
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![
            outbox_migration("m0001_create_events", "notes"),
            autoxide_kernel::event::outbox_actor_migration("m0002_add_event_actor", "notes"),
            autoxide_kernel::tenancy::migration::adopt(
                "m0003_tenancy",
                vec![autoxide_kernel::event::tenant_table("notes")],
            ),
            autoxide_kernel::event::outbox_access_migration("k0001_event_access", "notes"),
        ]
    }

    fn events(&self) -> Vec<EventInfo> {
        vec![NOTE_CREATED]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        *self.0.lock().unwrap() = Some(context.outbox()?);
        Ok(())
    }
}

/// Bên nhận ghi lại id event nhận được. `fail`: lỗi mỗi lần; `fail_first`: lỗi lần đầu với mỗi event.
#[derive(Default)]
struct Recorder {
    seen: Mutex<Vec<Uuid>>,
    fail: AtomicBool,
    fail_first: bool,
    failed_once: Mutex<HashSet<Uuid>>,
}

impl Recorder {
    fn failing() -> Arc<Self> {
        Arc::new(Recorder { fail: AtomicBool::new(true), ..Recorder::default() })
    }

    fn count(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
}

#[async_trait::async_trait]
impl EventHandler for Recorder {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        self.seen.lock().unwrap().push(event.id);
        if self.fail.load(Ordering::SeqCst) {
            return Err(Error::internal("the downstream system is down"));
        }
        if self.fail_first && self.failed_once.lock().unwrap().insert(event.id) {
            return Err(Error::internal("a hiccup"));
        }
        Ok(())
    }
}

/// Module nhận mọi event của `notes`.
struct Listener {
    name: &'static str,
    recorder: Arc<Recorder>,
}

impl Module for Listener {
    fn name(&self) -> &'static str {
        self.name
    }

    fn subscriptions(&self) -> Vec<Subscription> {
        vec![Subscription::new("notes.note_created", self.recorder.clone())]
    }
}

struct Setup {
    database: Database,
    modules: Modules,
    outbox: Outbox,
}

/// Dựng `notes` + hai module nhận theo thứ tự `alpha` rồi `beta`.
async fn setup(test: &TestDatabase, alpha: Arc<Recorder>, beta: Arc<Recorder>) -> Setup {
    let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
    let mut reader = ConfigReader::new(&source);
    let config = DatabaseConfig::read(&mut reader);
    let database = Database::connect(&reader.finish(config).unwrap().0).await.unwrap();
    let slot = Arc::new(Mutex::new(None));
    let mut modules = Modules::new(vec![
        Box::new(Notes(slot.clone())),
        Box::new(Listener { name: "alpha", recorder: alpha }),
        Box::new(Listener { name: "beta", recorder: beta }),
    ])
    .unwrap();
    modules.migrator().unwrap().up(&database).await.unwrap();
    modules.start(&database).unwrap();
    let outbox = slot.lock().unwrap().clone().unwrap();
    Setup { database, modules, outbox }
}

async fn record(setup: &Setup, title: &str) {
    let transaction = setup.database.begin_write().await.unwrap();
    setup
        .outbox
        .record(
            &transaction,
            &TenantContext::default_on(setup.database.clone()),
            Actor::system(),
            None,
            &NoteCreated { title: title.to_owned() },
        )
        .await
        .unwrap();
    transaction.commit().await.unwrap();
}

async fn execute(setup: &Setup, sql: &str) {
    let backend = setup.database.write().get_database_backend();
    setup.database.write().execute_raw(Statement::from_string(backend, sql)).await.unwrap();
}

/// Cho mọi lần thử lại theo bên nhận tới hạn ngay.
async fn retries_due(setup: &Setup) {
    execute(setup, "UPDATE kernel_event_deliveries SET next_attempt_at = created_at").await;
}

/// Các dòng thử lại: (bên nhận, số lần thử, đã giao, đã vào dead-letter).
async fn retries(setup: &Setup) -> Vec<(String, i32, bool, bool)> {
    let backend = setup.database.read().get_database_backend();
    let sql = "SELECT subscriber, attempts, delivered_at, failed_at FROM kernel_event_deliveries \
               ORDER BY id";
    let rows =
        setup.database.read().query_all_raw(Statement::from_string(backend, sql)).await.unwrap();
    rows.iter()
        .map(|row| {
            (
                row.try_get("", "subscriber").unwrap(),
                row.try_get("", "attempts").unwrap(),
                row.try_get::<Option<Timestamp>>("", "delivered_at").unwrap().is_some(),
                row.try_get::<Option<Timestamp>>("", "failed_at").unwrap().is_some(),
            )
        })
        .collect()
}

async fn count(setup: &Setup, sql: &str) -> i64 {
    let backend = setup.database.read().get_database_backend();
    let row = setup
        .database
        .read()
        .query_one_raw(Statement::from_string(backend, sql))
        .await
        .unwrap()
        .unwrap();
    row.try_get("", "n").unwrap()
}

/// `alpha` (đăng ký trước) lỗi: `beta` vẫn nhận ngay, đúng một lần; chỉ `alpha` thử lại, và khi hệ thống của nó
/// lành lại thì nó nhận — `beta` không bị gọi lại.
#[tokio::test]
async fn ben_truoc_loi_ben_sau_van_nhan_dung_mot_lan() {
    TestDatabase::run(async |test| {
        let (alpha, beta) = (Recorder::failing(), Arc::new(Recorder::default()));
        let setup = setup(test, alpha.clone(), beta.clone()).await;
        record(&setup, "first").await;
        let dispatcher = setup.modules.dispatcher().unwrap();

        assert_eq!(dispatcher.run_once().await.unwrap(), 1);
        assert_eq!((alpha.count(), beta.count()), (1, 1));
        assert_eq!(retries(&setup).await, [("alpha".to_owned(), 1, false, false)]);
        let undelivered = "SELECT COUNT(*) AS n FROM notes_events WHERE delivered_at IS NULL";
        assert_eq!(count(&setup, undelivered).await, 0, "the fan-out is finished");
        // Tồn đọng của `notes` đếm cả lần giao còn chờ của bên nhận.
        let snapshot =
            backlog(&setup.database, "main", &[], &["notes"], Timestamp::now()).await.unwrap();
        assert_eq!(snapshot.modules[0].events_pending, 1);
        assert_eq!(snapshot.modules[0].events_dead, 0);

        // Chưa tới hạn thử lại: không ai được gọi.
        assert_eq!(dispatcher.run_once().await.unwrap(), 0);
        // Tới hạn, vẫn lỗi: chỉ `alpha` được gọi.
        retries_due(&setup).await;
        assert_eq!(dispatcher.run_once().await.unwrap(), 1);
        assert_eq!((alpha.count(), beta.count()), (2, 1));
        assert_eq!(retries(&setup).await, [("alpha".to_owned(), 2, false, false)]);
        // Hệ thống của `alpha` lành lại.
        alpha.fail.store(false, Ordering::SeqCst);
        retries_due(&setup).await;
        assert_eq!(dispatcher.run_once().await.unwrap(), 1);
        assert_eq!((alpha.count(), beta.count()), (3, 1));
        assert_eq!(retries(&setup).await, [("alpha".to_owned(), 2, true, false)]);
        assert_eq!(dispatcher.run_once().await.unwrap(), 0);
    })
    .await
    .unwrap();
}

/// Bên lỗi mãi: riêng dòng của nó vào dead-letter sau `MAX_ATTEMPTS` lần; bên kia đã nhận từ đầu, event mới vẫn
/// tới cả hai không chờ.
#[tokio::test]
async fn dead_letter_rieng_cho_tung_ben() {
    TestDatabase::run(async |test| {
        let (alpha, beta) = (Arc::new(Recorder::default()), Recorder::failing());
        let setup = setup(test, alpha.clone(), beta.clone()).await;
        record(&setup, "doomed").await;
        let dispatcher = setup.modules.dispatcher().unwrap();
        assert_eq!(dispatcher.run_once().await.unwrap(), 1);
        for _ in 2..=MAX_ATTEMPTS {
            retries_due(&setup).await;
            assert_eq!(dispatcher.run_once().await.unwrap(), 1);
        }
        retries_due(&setup).await;
        assert_eq!(dispatcher.run_once().await.unwrap(), 0);
        assert_eq!((alpha.count(), beta.count()), (1, MAX_ATTEMPTS as usize));
        assert_eq!(retries(&setup).await, [("beta".to_owned(), MAX_ATTEMPTS, false, true)]);
        let snapshot =
            backlog(&setup.database, "main", &[], &["notes"], Timestamp::now()).await.unwrap();
        assert_eq!((snapshot.modules[0].events_pending, snapshot.modules[0].events_dead), (0, 1));

        // Event sau tới cả hai bên ngay lượt đầu.
        record(&setup, "next").await;
        assert_eq!(dispatcher.run_once().await.unwrap(), 1);
        assert_eq!((alpha.count(), beta.count()), (2, MAX_ATTEMPTS as usize + 1));
    })
    .await
    .unwrap();
}

/// Hai bộ phát (như hai máy chủ) chia và thử lại cùng lúc: mỗi bên nhận mỗi event đúng như số lần cần — `beta`
/// đúng một lần, `alpha` (lỗi lần đầu) đúng hai lần (lần lỗi và lần thử lại).
#[tokio::test]
async fn nhieu_bo_phat_dong_thoi_moi_ben_khong_nhan_trung() {
    TestDatabase::run(async |test| {
        let alpha = Arc::new(Recorder { fail_first: true, ..Recorder::default() });
        let beta = Arc::new(Recorder::default());
        let setup = setup(test, alpha.clone(), beta.clone()).await;
        for index in 0..30 {
            record(&setup, &format!("note-{index}")).await;
        }
        let first = setup.modules.dispatcher().unwrap();
        let second = setup.modules.dispatcher().unwrap();

        let (a, b) = tokio::join!(first.run_once(), second.run_once());
        assert_eq!(a.unwrap() + b.unwrap(), 30);
        retries_due(&setup).await;
        let (a, b) = tokio::join!(first.run_once(), second.run_once());
        assert_eq!(a.unwrap() + b.unwrap(), 30);
        let (a, b) = tokio::join!(first.run_once(), second.run_once());
        assert_eq!(a.unwrap() + b.unwrap(), 0);

        let tally = |recorder: &Recorder| {
            let mut counts: HashMap<Uuid, usize> = HashMap::new();
            for id in recorder.seen.lock().unwrap().iter() {
                *counts.entry(*id).or_default() += 1;
            }
            counts
        };
        let (alpha, beta) = (tally(&alpha), tally(&beta));
        assert_eq!((alpha.len(), beta.len()), (30, 30));
        assert!(alpha.values().all(|count| *count == 2), "{alpha:?}");
        assert!(beta.values().all(|count| *count == 1), "{beta:?}");
        let rows = retries(&setup).await;
        assert_eq!(rows.len(), 30);
        assert!(rows.iter().all(|row| row == &("alpha".to_owned(), 1, true, false)), "{rows:?}");
    })
    .await
    .unwrap();
}

/// Nâng cấp từ bản cũ: event đang chờ thử lại theo cách cũ (cả event, `attempts` > 0) được chia lại cho mọi bên —
/// không mất; event đã hỏng hẳn theo cách cũ (`failed_at`) nằm yên trong dead-letter.
#[tokio::test]
async fn nang_cap_tu_du_lieu_cu() {
    TestDatabase::run(async |test| {
        let (alpha, beta) = (Recorder::failing(), Arc::new(Recorder::default()));
        let setup = setup(test, alpha.clone(), beta.clone()).await;
        record(&setup, "waiting").await;
        record(&setup, "dead").await;
        // Trạng thái bộ phát cũ để lại: lần đầu `alpha` lỗi nên `beta` chưa nhận; event chờ lần thử thứ 4. Event kia
        // đã quá số lần thử.
        execute(
            &setup,
            "UPDATE notes_events SET attempts = 3, last_error = 'module `alpha`: down', \
             next_attempt_at = occurred_at WHERE payload LIKE '%waiting%'",
        )
        .await;
        execute(
            &setup,
            "UPDATE notes_events SET attempts = 10, last_error = 'module `alpha`: down', \
             failed_at = occurred_at WHERE payload LIKE '%dead%'",
        )
        .await;

        let dispatcher = setup.modules.dispatcher().unwrap();
        assert_eq!(dispatcher.run_once().await.unwrap(), 1);
        assert_eq!((alpha.count(), beta.count()), (1, 1));
        assert_eq!(retries(&setup).await, [("alpha".to_owned(), 1, false, false)]);
        let dead = "SELECT COUNT(*) AS n FROM notes_events WHERE failed_at IS NOT NULL \
                    AND delivered_at IS NULL";
        assert_eq!(count(&setup, dead).await, 1);
        assert_eq!(dispatcher.run_once().await.unwrap(), 0);
    })
    .await
    .unwrap();
}

/// Dọn event cũ: event còn bên phải nhận lại thì giữ (lần thử lại cần đọc nó), event đã xong thì xoá.
#[tokio::test]
async fn don_event_giu_event_con_ben_chua_nhan() {
    TestDatabase::run(async |test| {
        let alpha = Arc::new(Recorder { fail_first: true, ..Recorder::default() });
        let beta = Arc::new(Recorder::default());
        let setup = setup(test, alpha.clone(), beta.clone()).await;
        record(&setup, "kept").await;
        let dispatcher = setup.modules.dispatcher().unwrap();
        assert_eq!(dispatcher.run_once().await.unwrap(), 1);
        // Event đã chia xong từ 8 ngày trước.
        let old = Timestamp::now() - SignedDuration::from_hours(24 * 8);
        let age = Query::update()
            .table(Alias::new("notes_events"))
            .value(Alias::new("delivered_at"), old)
            .and_where(Expr::col(Alias::new("delivered_at")).is_not_null())
            .to_owned();
        setup.database.write().execute(&age).await.unwrap();

        // Bộ phát mới dọn ở lượt đầu: event còn `alpha` chờ nên còn, và lần thử lại vẫn đọc được nó.
        let fresh = setup.modules.dispatcher().unwrap();
        retries_due(&setup).await;
        assert_eq!(fresh.run_once().await.unwrap(), 1);
        assert_eq!(count(&setup, "SELECT COUNT(*) AS n FROM notes_events").await, 1);
        assert_eq!((alpha.count(), beta.count()), (2, 1));
        assert_eq!(retries(&setup).await, [("alpha".to_owned(), 1, true, false)]);

        // Xong cả rồi: lần dọn sau xoá event (và dòng thử lại đã giao cũ).
        let old_retry = Query::update()
            .table(Alias::new("kernel_event_deliveries"))
            .value(Alias::new("delivered_at"), old)
            .to_owned();
        setup.database.write().execute(&old_retry).await.unwrap();
        let later = setup.modules.dispatcher().unwrap();
        assert_eq!(later.run_once().await.unwrap(), 0);
        assert_eq!(count(&setup, "SELECT COUNT(*) AS n FROM notes_events").await, 0);
        assert!(retries(&setup).await.is_empty());
    })
    .await
    .unwrap();
}
