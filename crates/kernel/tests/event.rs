//! Event theo mẫu transactional outbox, trên database gate đang chọn (SQLite hoặc
//! Postgres): ghi cùng transaction, giao sau khi commit, giao lại khi lỗi, nhiều bộ
//! phát chạy cùng lúc không giao trùng.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

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
        vec![outbox_migration("m0001_create_events", "notes")]
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
}

#[async_trait::async_trait]
impl EventHandler for Recorder {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
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
        .record(&transaction, actor, Some(format!("notes.note:{title}")), &event)
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
        setup.outbox.record(&transaction, Actor::system(), None, &event).await.unwrap();
        transaction.rollback().await.unwrap();
        record(&setup, "first").await;

        let dispatcher = setup.modules.dispatcher(&setup.database).unwrap();
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

#[tokio::test]
async fn ben_nhan_loi_thi_giao_lai_roi_danh_dau_hong() {
    TestDatabase::run(async |test| {
        let recorder = Arc::new(Recorder { fail: true, ..Recorder::default() });
        let listener = Listener { event: Subscription::ALL, recorder: recorder.clone() };
        let setup = setup(test, listener).await;
        record(&setup, "doomed").await;
        let dispatcher = setup.modules.dispatcher(&setup.database).unwrap();

        for attempt in 1..=MAX_ATTEMPTS {
            assert_eq!(dispatcher.run_once().await.unwrap(), 1, "attempt {attempt}");
            // Lần sau được hẹn trong tương lai: chưa tới giờ thì không giao.
            assert_eq!(dispatcher.run_once().await.unwrap(), 0);
            // Cho thời gian trôi qua tới giờ thử lại.
            let sql = "UPDATE notes_events SET next_attempt_at = occurred_at";
            let backend = setup.database.write().get_database_backend();
            setup.database.write().execute_raw(Statement::from_string(backend, sql)).await.unwrap();
        }

        // Quá số lần thử: đánh dấu hỏng, không giao nữa.
        assert_eq!(dispatcher.run_once().await.unwrap(), 0);
        assert_eq!(recorder.seen.lock().unwrap().len(), MAX_ATTEMPTS as usize);
        let backend = setup.database.write().get_database_backend();
        let row = setup
            .database
            .read()
            .query_one_raw(Statement::from_string(
                backend,
                "SELECT attempts, last_error FROM notes_events",
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get::<i32>("", "attempts").unwrap(), MAX_ATTEMPTS);
        let error: String = row.try_get("", "last_error").unwrap();
        assert!(error.contains("module `listener`"), "{error}");
        assert!(error.contains("the downstream system is down"), "{error}");
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

        let first = setup.modules.dispatcher(&setup.database).unwrap();
        let second = setup.modules.dispatcher(&setup.database).unwrap();
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

        let error = setup.modules.dispatcher(&setup.database).err().unwrap().to_string();
        assert!(error.contains("subscribes to event `notes.nothing`"), "{error}");

        // Ghi event của module khác qua outbox của `notes` là lỗi của người viết code.
        #[derive(Serialize)]
        struct Foreign;
        impl Event for Foreign {
            const NAME: &'static str = "tags.tag_created";
        }
        let error =
            setup.outbox.record(setup.database.write(), Actor::system(), None, &Foreign).await;
        assert_eq!(error.unwrap_err().code(), "internal_error");
    })
    .await
    .unwrap();
}
