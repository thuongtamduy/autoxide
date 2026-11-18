//! Stream event thời gian thực bằng SSE (bước 40 P5, D-378): chỉ event khai `streamable`, lọc theo
//! permission, khách và tên; nối lại bằng `Last-Event-ID` không mất event; event commit muộn vẫn tới;
//! hai máy chủ chung một database; giới hạn kết nối; không bị lớp timeout cắt; đóng khi tắt. Chạy
//! trên cả SQLite và Postgres.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use autoxide_kernel::access::{Permission, Scope};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Event, EventInfo, Outbox, StreamHub};
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::tenancy::{OrgId, TenantContext};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt as _;
use serde::Serialize;
use tower::ServiceExt as _;
use uuid::Uuid;

const NOTES_READ: Permission = Permission::new("notes.read", "Read notes.").scoped();
const SECRETS_READ: Permission = Permission::new("notes.secrets_read", "Read secret notes.");

#[derive(Serialize)]
struct NoteCreated {
    title: String,
}

impl Event for NoteCreated {
    const NAME: &'static str = "notes.note_created";
}

#[derive(Serialize)]
struct SecretCreated {
    title: String,
}

impl Event for SecretCreated {
    const NAME: &'static str = "notes.secret_created";
}

#[derive(Serialize)]
struct NoteTouched {}

impl Event for NoteTouched {
    const NAME: &'static str = "notes.note_touched";
}

/// Module phát ba event: hai stream được (khác permission), một không.
struct Notes(Arc<Mutex<Option<Outbox>>>);

impl Module for Notes {
    fn name(&self) -> &'static str {
        "notes"
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![
            autoxide_kernel::event::outbox_migration("m0001_create_events", "notes"),
            autoxide_kernel::event::outbox_actor_migration("m0002_add_event_actor", "notes"),
            autoxide_kernel::tenancy::migration::adopt(
                "m0003_tenancy",
                vec![autoxide_kernel::event::tenant_table("notes")],
            ),
            autoxide_kernel::event::outbox_access_migration("k0001_event_access", "notes"),
        ]
    }

    fn permissions(&self) -> Vec<Permission> {
        vec![NOTES_READ, SECRETS_READ]
    }

    fn events(&self) -> Vec<EventInfo> {
        vec![
            EventInfo::new("notes.note_created", "A note was created.").streamable(NOTES_READ),
            EventInfo::new("notes.secret_created", "A secret note was created.")
                .streamable(SECRETS_READ),
            EventInfo::new("notes.note_touched", "A note was opened."),
        ]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        *self.0.lock().unwrap() = Some(context.outbox()?);
        Ok(())
    }
}

/// Một "máy chủ" trên database của test, cùng outbox của module `notes`.
struct Server {
    app: TestApp,
    outbox: Outbox,
}

async fn server(test: &TestDatabase, settings: &[(&str, &str)]) -> Server {
    build(test, settings, true).await
}

/// `first`: máy chủ đầu tiên trên database này (tạo khách `other`).
async fn build(test: &TestDatabase, settings: &[(&str, &str)], first: bool) -> Server {
    let slot = Arc::new(Mutex::new(None));
    let mut builder = TestApp::builder(test)
        .setting("AUTOXIDE_EVENTS_STREAM_POLL_MS", "100")
        .module(Notes(slot.clone()))
        .user("ada", 0xada, &["notes.read", "notes.secrets_read"])
        .user("bob", 0xb0b, &["notes.read"])
        .user("nobody", 0xdead, &[])
        .member("unit", 0x7, OrgId::from_uuid(Uuid::from_u128(0x7)), &[("notes.read", Scope::Unit)])
        .multi_tenant();
    if first {
        builder = builder.tenant("other").user_in("other", "eve", 0xe7e, &["notes.read"]);
    }
    if !settings.iter().any(|(name, _)| *name == "AUTOXIDE_EVENTS_STREAM") {
        builder = builder.setting("AUTOXIDE_EVENTS_STREAM", "true");
    }
    for (name, value) in settings {
        builder = builder.setting(name, value);
    }
    let app = builder.build().await.unwrap();
    let outbox = slot.lock().unwrap().clone().unwrap();
    Server { app, outbox }
}

impl Server {
    /// Ghi một event (commit ngay) cho khách `tenant`, do người dùng `actor` gây ra.
    async fn record<E: Event>(&self, tenant: &TenantContext, actor: u128, event: &E) {
        let transaction = tenant.database().begin_write().await.unwrap();
        let actor = Actor::user(Uuid::from_u128(actor));
        self.outbox.record(&transaction, tenant, actor, None, event).await.unwrap();
        transaction.commit().await.unwrap();
    }

    async fn note(&self, title: &str) {
        let tenant = self.app.default_tenant();
        self.record(&tenant, 0xada, &NoteCreated { title: title.to_owned() }).await;
    }

    /// Mở stream; trả mã trạng thái và (khi 200) thân để đọc dần.
    async fn open(&self, token: &str, query: &str, last: Option<&str>) -> (StatusCode, Stream) {
        let mut request = Request::get(format!("/api/v1/kernel/events/stream?{query}"))
            .header("authorization", format!("Bearer {token}"));
        if let Some(last) = last {
            request = request.header("last-event-id", last);
        }
        let response = self.app.router().oneshot(request.body(Body::empty()).unwrap()).await;
        let response = response.unwrap();
        let status = response.status();
        if status == StatusCode::OK {
            assert_eq!(response.headers()["content-type"], "text/event-stream");
            assert_eq!(response.headers()["x-accel-buffering"], "no");
        }
        (status, Stream { body: response.into_body(), buffer: String::new() })
    }
}

/// Một khung SSE đã đọc.
#[derive(Debug, Clone)]
struct Frame {
    id: Option<String>,
    event: Option<String>,
    data: serde_json::Value,
}

struct Stream {
    body: Body,
    buffer: String,
}

impl Stream {
    /// Khung event kế tiếp (bỏ qua chú thích và `retry`), hoặc `None` sau `wait` / khi stream đóng.
    async fn next(&mut self, wait: Duration) -> Option<Frame> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            if let Some(end) = self.buffer.find("\n\n") {
                let block: String = self.buffer.drain(..end + 2).collect();
                let mut frame = Frame { id: None, event: None, data: serde_json::Value::Null };
                for line in block.lines() {
                    if let Some(value) = line.strip_prefix("id: ") {
                        frame.id = Some(value.to_owned());
                    } else if let Some(value) = line.strip_prefix("event: ") {
                        frame.event = Some(value.to_owned());
                    } else if let Some(value) = line.strip_prefix("data: ") {
                        frame.data = serde_json::from_str(value).unwrap();
                    }
                }
                if frame.event.is_some() {
                    return Some(frame);
                }
                continue;
            }
            let frame = tokio::time::timeout_at(deadline, self.body.frame()).await.ok()??.ok()?;
            if let Ok(data) = frame.into_data() {
                self.buffer.push_str(&String::from_utf8_lossy(&data));
            }
        }
    }

    /// Tên các event nhận trong `wait`.
    async fn titles(&mut self, wait: Duration) -> Vec<String> {
        let mut titles = Vec::new();
        while let Some(frame) = self.next(wait).await {
            titles.push(frame.data["payload"]["title"].as_str().unwrap_or("?").to_owned());
        }
        titles
    }
}

const SHORT: Duration = Duration::from_millis(1500);

#[tokio::test]
async fn stream_nhan_event_duoc_phep() {
    TestDatabase::run(async |test| {
        let server = server(test, &[]).await;
        let (status, mut ada) = server.open("ada", "names=notes.*", None).await;
        assert_eq!(status, StatusCode::OK);
        let (_, mut bob) = server.open("bob", "names=notes.*", None).await;
        let tenant = server.app.default_tenant();

        server.note("first").await;
        server.record(&tenant, 0xada, &SecretCreated { title: "secret".into() }).await;
        server.record(&tenant, 0xada, &NoteTouched {}).await;

        let frame = ada.next(SHORT * 2).await.expect("an event");
        assert_eq!(frame.event.as_deref(), Some("notes.note_created"));
        assert_eq!(frame.id.as_deref(), frame.data["id"].as_str());
        assert_eq!(frame.data["name"], "notes.note_created");
        assert_eq!(frame.data["actor_kind"], "user");
        assert!(frame.data["occurred_at"].is_string());
        let frame = ada.next(SHORT).await.expect("the secret event");
        assert_eq!(frame.data["payload"]["title"], "secret");
        // Event không khai `streamable` không bao giờ ra ngoài.
        assert!(ada.next(SHORT).await.is_none());
        // Bob không có `notes.secrets_read`: chỉ thấy event đầu.
        assert_eq!(bob.titles(SHORT).await, ["first"]);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khong_nhan_event_khong_co_quyen() {
    TestDatabase::run(async |test| {
        let server = server(test, &[]).await;
        let (status, _) = server.open("nobody", "names=notes.note_created", None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) = server.open("bob", "names=notes.secret_created", None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) = server.open("nobody", "names=notes.*", None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        for query in ["names=notes.note_touched", "names=notes.nothing", "names=", "names=x.*"] {
            let (status, _) = server.open("ada", query, None).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{query}");
        }
        let (status, _) = server.open("ada", "names=notes.*", Some("not-an-id")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let request = Request::get("/api/v1/kernel/events/stream?names=notes.*");
        let response = server.app.router().oneshot(request.body(Body::empty()).unwrap()).await;
        // Không đăng nhập (chế độ nhiều khách: chưa biết khách nào): lỗi phía client, không có stream.
        assert!(response.unwrap().status().is_client_error());
    })
    .await
    .unwrap();
}

/// Quyền hẹp hơn cả khách (độ sâu `unit`): event chưa mang nút, nên chỉ thấy việc của mình.
#[tokio::test]
async fn quyen_hep_chi_nhan_viec_cua_minh() {
    TestDatabase::run(async |test| {
        let server = server(test, &[]).await;
        let (status, mut unit) = server.open("unit", "names=notes.note_created", None).await;
        assert_eq!(status, StatusCode::OK);
        let tenant = server.app.default_tenant();
        server.note("by ada").await;
        server.record(&tenant, 0x7, &NoteCreated { title: "mine".into() }).await;
        assert_eq!(unit.titles(SHORT * 2).await, ["mine"]);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khong_nhan_event_cua_khach_khac() {
    TestDatabase::run(async |test| {
        let server = server(test, &[]).await;
        let (_, mut bob) = server.open("bob", "names=notes.*", None).await;
        let (_, mut eve) = server.open("eve", "names=notes.*", None).await;
        let other = server.app.tenant("other").await.unwrap();

        server.record(&other, 0xe7e, &NoteCreated { title: "theirs".into() }).await;
        server.note("ours").await;

        assert_eq!(bob.titles(SHORT * 2).await, ["ours"]);
        assert_eq!(eve.titles(SHORT).await, ["theirs"]);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn noi_lai_bang_last_event_id_khong_mat_event() {
    TestDatabase::run(async |test| {
        let server = server(test, &[]).await;
        let (_, mut first) = server.open("bob", "names=notes.*", None).await;
        server.note("one").await;
        let one = first.next(SHORT * 2).await.unwrap();
        drop(first);

        // Mất kết nối: hai event ghi trong lúc đó.
        server.note("two").await;
        server.note("three").await;
        let (status, mut again) = server.open("bob", "names=notes.*", one.id.as_deref()).await;
        assert_eq!(status, StatusCode::OK);
        server.note("four").await;

        let titles = again.titles(SHORT * 2).await;
        assert_eq!(titles, ["two", "three", "four"]);

        // Cũng qua tham số `last_event_id` (client không đặt được header).
        let query = format!("names=notes.*&last_event_id={}", one.id.as_deref().unwrap());
        let (_, mut by_query) = server.open("bob", &query, None).await;
        assert_eq!(by_query.titles(SHORT).await, ["two", "three", "four"]);

        // ID cũ hơn khoảng giữ event: `reset`, client tải lại qua REST.
        let old = Uuid::new_v7(uuid::Timestamp::from_unix(uuid::NoContext, 1_000_000_000, 0));
        let (_, mut stale) = server.open("bob", "names=notes.*", Some(&old.to_string())).await;
        let reset = stale.next(SHORT).await.unwrap();
        assert_eq!(reset.event.as_deref(), Some("reset"));
    })
    .await
    .unwrap();
}

/// Transaction ghi event trước nhưng commit sau một event khác: ID (thời điểm ghi) sớm hơn event đã
/// phát, vẫn phải tới (hub hỏi lùi lại một khoảng).
#[tokio::test]
async fn event_commit_muon_van_toi() {
    TestDatabase::run(async |test| {
        let server = server(test, &[]).await;
        let (_, mut bob) = server.open("bob", "names=notes.note_created", None).await;
        server.note("on time").await;
        assert_eq!(bob.next(SHORT * 2).await.unwrap().data["payload"]["title"], "on time");

        // Như một transaction ghi từ 2 giây trước, chỉ vừa commit.
        let late = Uuid::new_v7(uuid::Timestamp::from_unix(
            uuid::NoContext,
            u64::try_from(chrono::Utc::now().timestamp() - 2).unwrap(),
            0,
        ));
        insert_late(&server, late, "late").await;
        let frame = bob.next(SHORT * 2).await.expect("the late event");
        assert_eq!(frame.id.as_deref(), Some(late.to_string().as_str()));
        assert_eq!(frame.data["payload"]["title"], "late");
    })
    .await
    .unwrap();
}

async fn insert_late(server: &Server, id: Uuid, title: &str) {
    use sea_orm::ConnectionTrait as _;
    use sea_orm::sea_query::{Alias, Query};
    let tenant = server.app.default_tenant();
    let at = chrono::Utc::now() - chrono::Duration::seconds(2);
    let payload = serde_json::json!({ "title": title }).to_string();
    let mut insert = Query::insert();
    insert
        .into_table(Alias::new("notes_events"))
        .columns(
            [
                "tenant_id",
                "id",
                "name",
                "actor_user_id",
                "actor_kind",
                "actor_access",
                "payload",
                "occurred_at",
                "attempts",
                "next_attempt_at",
            ]
            .map(Alias::new),
        )
        .values([
            tenant.id().as_uuid().into(),
            id.into(),
            "notes.note_created".into(),
            Some(Uuid::from_u128(0xada)).into(),
            "user".into(),
            "direct".into(),
            payload.into(),
            at.into(),
            0i32.into(),
            at.into(),
        ])
        .unwrap();
    tenant.database().write().execute(&insert).await.unwrap();
}

/// Ghi qua máy chủ A, đọc stream ở máy chủ B: hub đọc từ bảng event, không từ bộ nhớ của A.
#[tokio::test]
async fn hai_may_chu_chung_database() {
    TestDatabase::run(async |test| {
        let a = server(test, &[]).await;
        let b = build(test, &[], false).await;
        let (_, mut on_b) = b.open("bob", "names=notes.*", None).await;
        a.note("from a").await;
        assert_eq!(on_b.titles(SHORT * 2).await, ["from a"]);
        assert_eq!(a.app.api::<StreamHub>().unwrap().open_streams(), 0);
        assert_eq!(b.app.api::<StreamHub>().unwrap().open_streams(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn qua_gioi_han_ket_noi_429() {
    TestDatabase::run(async |test| {
        let server = server(
            test,
            &[
                ("AUTOXIDE_EVENTS_STREAM_MAX_PER_USER", "1"),
                ("AUTOXIDE_EVENTS_STREAM_MAX_PER_TENANT", "2"),
            ],
        )
        .await;
        let (status, first) = server.open("bob", "names=notes.*", None).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = server.open("bob", "names=notes.*", None).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        let (status, _second) = server.open("ada", "names=notes.*", None).await;
        assert_eq!(status, StatusCode::OK);
        // Khách đã đủ 2 kết nối.
        let (status, _) = server.open("unit", "names=notes.*", None).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        // Đóng một kết nối thì chỗ được trả lại.
        drop(first);
        let hub = server.app.api::<StreamHub>().unwrap();
        for _ in 0..50 {
            if hub.open_streams() == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let (status, _) = server.open("bob", "names=notes.*", None).await;
        assert_eq!(status, StatusCode::OK);
    })
    .await
    .unwrap();
}

/// Lớp timeout chung (ở đây 1 giây) không cắt stream; tắt máy chủ thì stream đóng.
#[tokio::test]
async fn stream_khong_bi_timeout_va_dong_khi_tat() {
    TestDatabase::run(async |test| {
        let server = server(test, &[("AUTOXIDE_HTTP_REQUEST_TIMEOUT_SECONDS", "1")]).await;
        let (_, mut bob) = server.open("bob", "names=notes.*", None).await;
        tokio::time::sleep(Duration::from_millis(1500)).await;
        server.note("still here").await;
        assert_eq!(bob.next(SHORT * 2).await.unwrap().data["payload"]["title"], "still here");

        server.app.api::<StreamHub>().unwrap().close();
        let started = std::time::Instant::now();
        assert!(bob.next(Duration::from_secs(5)).await.is_none());
        assert!(started.elapsed() < Duration::from_secs(3), "the stream did not end on shutdown");
        let (status, _) = server.open("bob", "names=notes.*", None).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tat_mac_dinh() {
    TestDatabase::run(async |test| {
        let server = server(test, &[("AUTOXIDE_EVENTS_STREAM", "false")]).await;
        let (status, _) = server.open("bob", "names=notes.*", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    })
    .await
    .unwrap();
}
