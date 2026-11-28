//! Event theo mẫu transactional outbox, trên database gate đang chọn (SQLite hoặc
//! Postgres): ghi cùng transaction, giao sau khi commit, giao lại khi lỗi, nhiều bộ
//! phát chạy cùng lúc không giao trùng.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_kernel::time::Timestamp;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use autoxide_kernel::config::{ConfigReader, ConfigSource, DatabaseConfig};
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{
    Actor, Delivered, Event, EventHandler, EventInfo, MAX_ATTEMPTS, Outbox, Subscription,
    outbox_migration,
};
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, Modules, StartContext};
use autoxide_testkit::TestDatabase;
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

/// Module phát event; giữ `Outbox` lấy lúc khởi động để test dùng.
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
            // Đường người làm vào khách (bước 34, D-310).
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

/// Bên nhận ghi lại mọi event; `fail` thì lỗi mỗi lần.
#[derive(Default)]
struct Recorder {
    seen: Mutex<Vec<Delivered>>,
    fail: bool,
    /// Mỗi event chờ chừng này mili giây (bên nhận chậm).
    slow_ms: u64,
}

#[async_trait::async_trait]
impl EventHandler for Recorder {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        if event.payload["title"] == "bug" {
            panic!("the event handler has a bug");
        }
        if self.slow_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(self.slow_ms)).await;
        }
        self.seen.lock().unwrap().push(event.clone());
        if self.fail {
            return Err(Error::internal("the downstream system is down"));
        }
        Ok(())
    }
}

/// Module nhận event với bộ ghi `recorder`, theo tên `event`.
struct Listener {
    event: &'static str,
    recorder: Arc<Recorder>,
}

impl Module for Listener {
    fn name(&self) -> &'static str {
        "listener"
    }

    fn subscriptions(&self) -> Vec<Subscription> {
        vec![Subscription::new(self.event, self.recorder.clone())]
    }
}

struct Setup {
    database: Database,
    modules: Modules,
    outbox: Outbox,
}

async fn setup(test: &TestDatabase, listener: Listener) -> Setup {
    let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
    let mut reader = ConfigReader::new(&source);
    let config = DatabaseConfig::read(&mut reader);
    let database = Database::connect(&reader.finish(config).unwrap().0).await.unwrap();
    let slot = Arc::new(Mutex::new(None));
    let mut modules =
        Modules::new(vec![Box::new(Notes(slot.clone())), Box::new(listener)]).unwrap();
    modules.migrator().unwrap().up(&database).await.unwrap();
    modules.start(&database).unwrap();
    let outbox = slot.lock().unwrap().clone().unwrap();
    Setup { database, modules, outbox }
}

async fn record(setup: &Setup, title: &str) {
    let actor = Actor::user(Uuid::nil());
    let event = NoteCreated { title: title.to_owned() };
    let transaction = setup.database.begin_write().await.unwrap();
    setup
        .outbox
        .record(
            &transaction,
            &autoxide_kernel::tenancy::TenantContext::default_on(setup.database.clone()),
            actor,
            Some(format!("notes.note:{title}")),
            &event,
        )
        .await
        .unwrap();
    transaction.commit().await.unwrap();
}

#[tokio::test]
async fn event_chi_ton_tai_khi_transaction_commit_roi_duoc_giao_mot_lan() {
    TestDatabase::run(async |test| {
        let recorder = Arc::new(Recorder::default());
        let listener = Listener { event: "notes.note_created", recorder: recorder.clone() };
        let setup = setup(test, listener).await;

        // Transaction bị huỷ: không có event nào.
        let transaction = setup.database.begin_write().await.unwrap();
        let event = NoteCreated { title: "draft".to_owned() };
        setup
            .outbox
            .record(
                &transaction,
                &autoxide_kernel::tenancy::TenantContext::default_on(setup.database.clone()),
                Actor::system(),
                None,
                &event,
            )
            .await
            .unwrap();
        transaction.rollback().await.unwrap();
        record(&setup, "first").await;

        let dispatcher = setup.modules.dispatcher().unwrap();
        assert_eq!(dispatcher.run_once().await.unwrap(), 1);
        assert_eq!(dispatcher.run_once().await.unwrap(), 0);

        let seen = recorder.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].name, "notes.note_created");
        assert_eq!(seen[0].module, "notes");
        assert_eq!(seen[0].actor, Actor::user(Uuid::nil()));
        assert_eq!(seen[0].subject.as_deref(), Some("notes.note:first"));
        assert_eq!(seen[0].payload, serde_json::json!({"title": "first"}));
    })
    .await
    .unwrap();
}

/// Event do người lạ gây ra giữ loại người thực hiện qua outbox: khác hệ thống.
#[tokio::test]
async fn nguoi_la_khac_he_thong() {
    TestDatabase::run(async |test| {
        let recorder = Arc::new(Recorder::default());
        let listener = Listener { event: "notes.note_created", recorder: recorder.clone() };
        let setup = setup(test, listener).await;
        for actor in [Actor::public(), Actor::system()] {
            let transaction = setup.database.begin_write().await.unwrap();
            let event = NoteCreated { title: actor.kind().to_owned() };
            setup
                .outbox
                .record(
                    &transaction,
                    &autoxide_kernel::tenancy::TenantContext::default_on(setup.database.clone()),
                    actor,
                    None,
                    &event,
                )
                .await
                .unwrap();
            transaction.commit().await.unwrap();
        }
        let dispatcher = setup.modules.dispatcher().unwrap();
        assert_eq!(dispatcher.run_once().await.unwrap(), 2);
        let seen = recorder.seen.lock().unwrap();
        let kinds: Vec<(&str, bool)> =
            seen.iter().map(|event| (event.actor.kind(), event.actor.public)).collect();
        assert_eq!(kinds, [("public", true), ("system", false)]);
        assert_eq!(Actor::user(Uuid::nil()).kind(), "user");
    })
    .await
    .unwrap();
}

/// Cho mọi lần thử lại theo bên nhận tới hạn ngay (thay vì chờ backoff).
async fn retries_due(setup: &Setup) {
    let sql = "UPDATE kernel_event_deliveries SET next_attempt_at = created_at";
    let backend = setup.database.write().get_database_backend();
    setup.database.write().execute_raw(Statement::from_string(backend, sql)).await.unwrap();
}

/// Bên nhận lỗi: event vẫn được chia xong, chỉ bên đó thử lại với backoff; quá `MAX_ATTEMPTS` lần thì riêng
/// dòng của bên đó vào dead-letter (D-20261128-event-per-subscriber).
#[tokio::test]
async fn ben_nhan_loi_thi_giao_lai_roi_danh_dau_hong() {
    TestDatabase::run(async |test| {
        let recorder = Arc::new(Recorder { fail: true, ..Recorder::default() });
        let listener = Listener { event: Subscription::ALL, recorder: recorder.clone() };
        let setup = setup(test, listener).await;
        record(&setup, "doomed").await;
        let dispatcher = setup.modules.dispatcher().unwrap();

        // Lần chia đầu là lần thử thứ nhất.
        assert_eq!(dispatcher.run_once().await.unwrap(), 1);
        for attempt in 2..=MAX_ATTEMPTS {
            // Lần sau được hẹn trong tương lai: chưa tới giờ thì không giao.
            assert_eq!(dispatcher.run_once().await.unwrap(), 0);
            retries_due(&setup).await;
            assert_eq!(dispatcher.run_once().await.unwrap(), 1, "attempt {attempt}");
        }

        // Quá số lần thử: đánh dấu hỏng, không giao nữa.
        retries_due(&setup).await;
        assert_eq!(dispatcher.run_once().await.unwrap(), 0);
        assert_eq!(recorder.seen.lock().unwrap().len(), MAX_ATTEMPTS as usize);
        let backend = setup.database.write().get_database_backend();
        let row = setup
            .database
            .read()
            .query_one_raw(Statement::from_string(
                backend,
                "SELECT subscriber, attempts, failed_at, last_error FROM kernel_event_deliveries",
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get::<String>("", "subscriber").unwrap(), "listener");
        assert_eq!(row.try_get::<i32>("", "attempts").unwrap(), MAX_ATTEMPTS);
        assert!(row.try_get::<Option<Timestamp>>("", "failed_at").unwrap().is_some());
        let error: String = row.try_get("", "last_error").unwrap();
        assert!(error.contains("module `listener`"), "{error}");
        assert!(error.contains("the downstream system is down"), "{error}");
        // Event đã chia xong: không nằm chờ ở outbox nữa.
        let event = setup
            .database
            .read()
            .query_one_raw(Statement::from_string(
                backend,
                "SELECT delivered_at, failed_at FROM notes_events",
            ))
            .await
            .unwrap()
            .unwrap();
        assert!(event.try_get::<Option<Timestamp>>("", "delivered_at").unwrap().is_some());
        assert!(event.try_get::<Option<Timestamp>>("", "failed_at").unwrap().is_none());
    })
    .await
    .unwrap();
}

/// Event xoá tài khoản không bao giờ bị bỏ: quá số lần thử vẫn được giao lại (D-149), theo từng bên nhận.
#[tokio::test]
async fn event_xoa_tai_khoan_thu_lai_mai() {
    TestDatabase::run(async |test| {
        let recorder = Arc::new(Recorder { fail: true, ..Recorder::default() });
        let listener = Listener { event: Subscription::ALL, recorder: recorder.clone() };
        let setup = setup(test, listener).await;
        record(&setup, "erase me").await;
        let backend = setup.database.write().get_database_backend();
        let rename =
            format!("UPDATE notes_events SET name = '{}'", autoxide_kernel::privacy::USER_DELETED);
        setup.database.write().execute_raw(Statement::from_string(backend, rename)).await.unwrap();
        let dispatcher = setup.modules.dispatcher().unwrap();

        for attempt in 1..=MAX_ATTEMPTS + 2 {
            assert_eq!(dispatcher.run_once().await.unwrap(), 1, "attempt {attempt}");
            retries_due(&setup).await;
        }
        let row = setup
            .database
            .read()
            .query_one_raw(Statement::from_string(
                backend,
                "SELECT attempts, failed_at FROM kernel_event_deliveries",
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get::<i32>("", "attempts").unwrap(), MAX_ATTEMPTS + 2);
        let failed: Option<Timestamp> = row.try_get("", "failed_at").unwrap();
        assert!(failed.is_none());
    })
    .await
    .unwrap();
}

/// Hai bộ phát (như hai máy chủ) chạy cùng lúc: mỗi event được giao đúng một lần.
#[tokio::test]
async fn nhieu_bo_phat_dong_thoi_khong_giao_trung() {
    TestDatabase::run(async |test| {
        let recorder = Arc::new(Recorder::default());
        let listener = Listener { event: Subscription::ALL, recorder: recorder.clone() };
        let setup = setup(test, listener).await;
        for index in 0..30 {
            record(&setup, &format!("note-{index}")).await;
        }

        let first = setup.modules.dispatcher().unwrap();
        let second = setup.modules.dispatcher().unwrap();
        let (a, b) = tokio::join!(first.run_once(), second.run_once());

        assert_eq!(a.unwrap() + b.unwrap(), 30);
        let mut counts: HashMap<Uuid, usize> = HashMap::new();
        for event in recorder.seen.lock().unwrap().iter() {
            *counts.entry(event.id).or_default() += 1;
        }
        assert_eq!(counts.len(), 30);
        assert!(counts.values().all(|count| *count == 1), "{counts:?}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khai_sai_thi_bao_ro() {
    TestDatabase::run(async |test| {
        let listener = Listener { event: "notes.nothing", recorder: Arc::default() };
        let setup = setup(test, listener).await;

        let error = setup.modules.dispatcher().err().unwrap().to_string();
        assert!(error.contains("subscribes to event `notes.nothing`"), "{error}");
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(setup.database.clone());

        // Ghi event của module khác qua outbox của `notes` là lỗi của người viết code.
        #[derive(Serialize)]
        struct Foreign;
        impl Event for Foreign {
            const NAME: &'static str = "tags.tag_created";
        }
        let error = setup
            .outbox
            .record(setup.database.write(), &tenant, Actor::system(), None, &Foreign)
            .await;
        assert_eq!(error.unwrap_err().code(), "internal_error");
    })
    .await
    .unwrap();
}

/// Bên nhận panic, hay dòng event không đọc được: tính là giao lỗi / hỏng hẳn, các event sau
/// vẫn được giao — dispatcher không chết, không bị chặn (D-158).
#[tokio::test]
async fn ben_nhan_panic_hay_dong_hong_khong_chan_event_sau() {
    TestDatabase::run(async |test| {
        let recorder = Arc::new(Recorder::default());
        let listener = Listener { event: Subscription::ALL, recorder: recorder.clone() };
        let setup = setup(test, listener).await;
        record(&setup, "bug").await;
        record(&setup, "broken").await;
        record(&setup, "fine").await;
        // Dòng thứ hai hỏng: payload không phải JSON.
        use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Query};
        let damage = Query::update()
            .table(Alias::new("notes_events"))
            .value(Alias::new("payload"), "{not json")
            .and_where(Expr::col(Alias::new("subject")).eq("notes.note:broken"))
            .to_owned();
        setup.database.write().execute(&damage).await.unwrap();

        let dispatcher = setup.modules.dispatcher().unwrap();
        dispatcher.run_once().await.unwrap();
        let seen: Vec<String> = recorder
            .seen
            .lock()
            .unwrap()
            .iter()
            .map(|event| event.payload["title"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(seen, ["fine"]);
        let backend = setup.database.read().get_database_backend();
        let rows = setup
            .database
            .read()
            .query_all_raw(Statement::from_string(
                backend,
                "SELECT subject, failed_at FROM notes_events ORDER BY id",
            ))
            .await
            .unwrap();
        let broken = &rows[1];
        assert!(broken.try_get::<Option<Timestamp>>("", "failed_at").unwrap().is_some());
        // Bên nhận panic: một lần lỗi của riêng bên đó, chờ thử lại.
        let bug = setup
            .database
            .read()
            .query_one_raw(Statement::from_string(
                backend,
                "SELECT attempts, failed_at, last_error FROM kernel_event_deliveries",
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(bug.try_get::<i32>("", "attempts").unwrap(), 1);
        assert!(bug.try_get::<Option<Timestamp>>("", "failed_at").unwrap().is_none());
        let error: String = bug.try_get("", "last_error").unwrap();
        assert!(error.contains("panicked"), "{error}");
    })
    .await
    .unwrap();
}

/// Module nghe event của module có thể chưa cài (`Subscription::optional`, bước 41).
struct OptionalListener {
    recorder: Arc<Recorder>,
}

impl Module for OptionalListener {
    fn name(&self) -> &'static str {
        "listener"
    }

    fn subscriptions(&self) -> Vec<Subscription> {
        vec![
            Subscription::optional("orders.order_confirmed", self.recorder.clone()),
            Subscription::optional("notes.note_created", self.recorder.clone()),
        ]
    }
}

/// `Subscription::optional` cho event chưa module nào khai: khởi động không lỗi, và event của
/// module ĐÃ cài vẫn tới như `Subscription::new`.
#[tokio::test]
async fn subscription_optional_khong_loi_khoi_dong() {
    TestDatabase::run(async |test| {
        let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
        let mut reader = ConfigReader::new(&source);
        let config = DatabaseConfig::read(&mut reader);
        let database = Database::connect(&reader.finish(config).unwrap().0).await.unwrap();
        let slot = Arc::new(Mutex::new(None));
        let recorder = Arc::new(Recorder::default());
        let mut modules = Modules::new(vec![
            Box::new(Notes(slot.clone())),
            Box::new(OptionalListener { recorder: recorder.clone() }),
        ])
        .unwrap();
        modules.migrator().unwrap().up(&database).await.unwrap();
        modules.start(&database).unwrap();
        let outbox = slot.lock().unwrap().clone().unwrap();

        let dispatcher = modules.dispatcher().unwrap();
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        outbox
            .record(
                database.write(),
                &tenant,
                Actor::system(),
                None,
                &NoteCreated { title: "optional".into() },
            )
            .await
            .unwrap();
        assert_eq!(dispatcher.run_once().await.unwrap(), 1);
        assert_eq!(recorder.seen.lock().unwrap().len(), 1);
    })
    .await
    .unwrap();
}

/// PT-17: lệnh dừng tới khi còn tồn đọng lớn thì bộ phát dừng sau lượt đang chạy, không giao hết rồi mới dừng (trước
/// đây tiến trình sống thêm hàng chục giây sau SIGTERM). Event chưa giao nằm lại cho lần chạy sau.
#[tokio::test]
async fn lenh_dung_khong_doi_giao_het_ton_dong() {
    TestDatabase::run(async |test| {
        let recorder = Arc::new(Recorder { slow_ms: 5, ..Recorder::default() });
        let listener = Listener { event: Subscription::ALL, recorder: recorder.clone() };
        let setup = setup(test, listener).await;
        for index in 0..250 {
            record(&setup, &format!("note-{index}")).await;
        }
        let dispatcher = setup.modules.dispatcher().unwrap();
        // Lệnh dừng đã có từ đầu: một lượt (một lô) rồi dừng.
        dispatcher.run(std::future::ready(())).await;
        let delivered = recorder.seen.lock().unwrap().len();
        assert!(delivered > 0 && delivered < 250, "delivered {delivered} of 250 before stopping");
        // Phần còn lại vẫn được giao ở lần chạy sau.
        let rest = setup.modules.dispatcher().unwrap();
        while rest.run_once().await.unwrap() > 0 {}
        assert_eq!(recorder.seen.lock().unwrap().len(), 250);
    })
    .await
    .unwrap();
}
