//! WebSocket của event thời gian thực (bước 69 P3): cùng event, quyền và cách nối lại như SSE; nối
//! lại không mất event; hai máy chủ chung một database; token hết hạn thì máy chủ đóng (4401); máy
//! khách chỉ gửi được lệnh điều khiển, không ghi được dữ liệu; tín hiệu "đang gõ" / "online" chỉ
//! trong chủ đề module cho vào. Chạy trên cả SQLite và Postgres.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use autoxide_kernel::access::{Authenticator, Permission, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Event, EventInfo, Outbox, SignalTopics};
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::tenancy::{DEFAULT_TENANT, TenantContext, TenantHint};
use autoxide_testkit::{TestApp, TestDatabase};
use futures_util::{SinkExt as _, StreamExt as _};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
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

/// Phòng `notes.room:<id>`: ai có `notes.read` vào được phòng `open`, không ai vào phòng khác.
struct Rooms;

#[async_trait::async_trait]
impl SignalTopics for Rooms {
    fn kind(&self) -> &'static str {
        "notes.room"
    }

    async fn allows(
        &self,
        _: &TenantContext,
        principal: &Principal,
        id: &str,
    ) -> Result<bool, Error> {
        Ok(id == "open" && principal.require(&NOTES_READ).is_ok())
    }
}

/// Module phát hai event stream được (khác permission) và mở chủ đề `notes.room`.
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
        ]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        *self.0.lock().unwrap() = Some(context.outbox()?);
        context.contribute::<dyn SignalTopics>(Arc::new(Rooms))
    }
}

/// Bộ xác thực có token thu hồi được (token hết hạn giữa chừng).
#[derive(Default)]
struct Tokens(Mutex<HashMap<String, Principal>>);

#[async_trait::async_trait]
impl Authenticator for Tokens {
    async fn authenticate(
        &self,
        authorization: Option<&str>,
        _: &TenantHint,
    ) -> Result<Principal, Error> {
        let token = authorization.and_then(|value| value.strip_prefix("Bearer ")).unwrap_or("");
        self.0.lock().unwrap().get(token).cloned().ok_or_else(|| {
            Error::unauthenticated("token_expired", "The access token expired. Sign in again.")
        })
    }
}

/// Một "máy chủ" thật (mở port) trên database của test.
struct Server {
    app: TestApp,
    outbox: Outbox,
    tokens: Arc<Tokens>,
    url: String,
}

async fn server(test: &TestDatabase) -> Server {
    let slot = Arc::new(Mutex::new(None));
    let tokens = Arc::new(Tokens::default());
    for (token, user, permissions) in [
        ("ada", 0xada, vec!["notes.read", "notes.secrets_read"]),
        ("bob", 0xb0b, vec!["notes.read"]),
    ] {
        let principal =
            Principal::new(Uuid::from_u128(user), permissions.into_iter().map(str::to_owned))
                .in_tenant(DEFAULT_TENANT);
        tokens.0.lock().unwrap().insert(token.to_owned(), principal);
    }
    let app = TestApp::builder(test)
        .setting("AUTOXIDE_EVENTS_STREAM", "true")
        .setting("AUTOXIDE_EVENTS_STREAM_POLL_MS", "100")
        .setting("AUTOXIDE_EVENTS_WS_RECHECK_SECONDS", "1")
        .provide::<dyn Authenticator>(tokens.clone())
        .module(Notes(slot.clone()))
        .build()
        .await
        .unwrap();
    let outbox = slot.lock().unwrap().clone().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = app.router();
    tokio::spawn(async move {
        let service = router.into_make_service_with_connect_info::<SocketAddr>();
        axum::serve(listener, service).await.unwrap();
    });
    Server { app, outbox, tokens, url: format!("ws://{address}/api/v1/kernel/events/ws") }
}

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

impl Server {
    async fn record<E: Event>(&self, event: &E) {
        let tenant = self.app.default_tenant();
        let transaction = tenant.database().begin_write().await.unwrap();
        let actor = Actor::user(Uuid::from_u128(0xada));
        self.outbox.record(&transaction, &tenant, actor, None, event).await.unwrap();
        transaction.commit().await.unwrap();
    }

    async fn note(&self, title: &str) {
        self.record(&NoteCreated { title: title.to_owned() }).await;
    }

    /// Bắt tay; `Err(status)` khi máy chủ từ chối bằng HTTP.
    async fn connect(&self, token: &str, query: &str) -> Result<Socket, u16> {
        let mut request = format!("{}?{query}", self.url).into_client_request().unwrap();
        request.headers_mut().insert("authorization", format!("Bearer {token}").parse().unwrap());
        match tokio_tungstenite::connect_async(request).await {
            Ok((mut socket, _)) => {
                let first = next(&mut socket, LONG).await.expect("the subscription");
                assert_eq!(first["type"], "subscribed", "{first}");
                Ok(socket)
            }
            Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
                Err(response.status().as_u16())
            }
            Err(error) => panic!("{error}"),
        }
    }
}

const SHORT: Duration = Duration::from_millis(1200);
const LONG: Duration = Duration::from_secs(5);

/// Thông điệp JSON kế tiếp trong `wait`; `None` khi hết giờ hay socket đóng.
async fn next(socket: &mut Socket, wait: Duration) -> Option<Value> {
    loop {
        match tokio::time::timeout(wait, socket.next()).await.ok()?? {
            Ok(Message::Text(body)) => return Some(serde_json::from_str(body.as_str()).unwrap()),
            Ok(Message::Close(_)) | Err(_) => return None,
            Ok(_) => {}
        }
    }
}

/// Tên các ghi chú nhận trong `wait`.
async fn titles(socket: &mut Socket, wait: Duration) -> Vec<String> {
    let mut titles = Vec::new();
    while let Some(message) = next(socket, wait).await {
        if message["type"] == "event" {
            titles.push(message["event"]["payload"]["title"].as_str().unwrap().to_owned());
        }
    }
    titles
}

async fn send(socket: &mut Socket, message: Value) {
    socket.send(Message::text(message.to_string())).await.unwrap();
}

#[tokio::test]
async fn nhan_dung_event_duoc_phep() {
    TestDatabase::run(async |test| {
        let server = server(test).await;
        let mut ada = server.connect("ada", "names=notes.*").await.unwrap();
        let mut bob = server.connect("bob", "names=notes.*").await.unwrap();
        server.note("first").await;
        server.record(&SecretCreated { title: "secret".into() }).await;

        let event = next(&mut ada, LONG).await.unwrap();
        assert_eq!(event["type"], "event");
        assert_eq!(event["event"]["name"], "notes.note_created");
        assert!(event["event"]["id"].is_string() && event["event"]["occurred_at"].is_string());
        assert_eq!(titles(&mut ada, SHORT).await, ["secret"]);
        assert_eq!(titles(&mut bob, SHORT * 2).await, ["first"]);

        // Bắt tay sai: như SSE.
        assert_eq!(server.connect("nobody", "names=notes.*").await.err(), Some(401));
        assert_eq!(server.connect("bob", "names=notes.secret_created").await.err(), Some(403));
        assert_eq!(server.connect("ada", "names=notes.nothing").await.err(), Some(422));
        // Đăng ký thêm event không có quyền: báo lỗi, giữ đăng ký cũ.
        send(&mut bob, json!({"type": "subscribe", "names": "notes.secret_created"})).await;
        let error = next(&mut bob, LONG).await.unwrap();
        assert_eq!(
            (error["type"].as_str(), error["code"].as_str()),
            (Some("error"), Some("permission_denied"))
        );
        server.note("second").await;
        assert_eq!(titles(&mut bob, SHORT * 2).await, ["second"]);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn noi_lai_va_doi_dang_ky_khong_mat_event() {
    TestDatabase::run(async |test| {
        let server = server(test).await;
        let mut ada = server.connect("ada", "names=notes.note_created").await.unwrap();
        server.note("one").await;
        let first = next(&mut ada, LONG).await.unwrap();
        let id = first["event"]["id"].as_str().unwrap().to_owned();
        drop(ada);
        server.note("two").await;
        server.note("three").await;
        let mut again = server
            .connect("ada", &format!("names=notes.note_created&last_event_id={id}"))
            .await
            .unwrap();
        assert_eq!(titles(&mut again, SHORT * 2).await, ["two", "three"]);

        // Đổi đăng ký giữa chừng: event ghi trong lúc đổi vẫn tới, theo tập mới.
        send(&mut again, json!({"type": "subscribe", "names": "notes.secret_created"})).await;
        server.record(&SecretCreated { title: "four".into() }).await;
        let subscribed = next(&mut again, LONG).await.unwrap();
        assert_eq!(
            subscribed,
            json!({"type": "subscribed", "names": ["notes.note_created", "notes.secret_created"]})
        );
        assert_eq!(titles(&mut again, SHORT * 2).await, ["four"]);
        send(&mut again, json!({"type": "unsubscribe", "names": "notes.note_created"})).await;
        assert_eq!(next(&mut again, LONG).await.unwrap()["names"], json!(["notes.secret_created"]));
        server.note("not for me").await;
        assert!(titles(&mut again, SHORT * 2).await.is_empty());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hai_may_chu_chung_database() {
    TestDatabase::run(async |test| {
        let a = server(test).await;
        let b = server(test).await;
        let mut on_b = b.connect("bob", "names=notes.*").await.unwrap();
        a.note("from a").await;
        assert_eq!(titles(&mut on_b, SHORT * 2).await, ["from a"]);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn token_het_han_thi_dong_4401() {
    TestDatabase::run(async |test| {
        let server = server(test).await;
        let mut bob = server.connect("bob", "names=notes.*").await.unwrap();
        server.tokens.0.lock().unwrap().remove("bob");
        let closed = tokio::time::timeout(LONG, async {
            loop {
                match bob.next().await {
                    Some(Ok(Message::Close(Some(frame)))) => return frame,
                    Some(Ok(_)) => {}
                    other => panic!("expected a close frame, got {other:?}"),
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(u16::from(closed.code), 4401);
        assert_eq!(closed.reason.as_str(), "token_expired");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khong_ghi_duoc_du_lieu_qua_websocket() {
    TestDatabase::run(async |test| {
        let server = server(test).await;
        let mut ada = server.connect("ada", "names=notes.*").await.unwrap();
        for message in [
            json!({"type": "create", "path": "/api/v1/notes", "body": {"title": "x"}}),
            json!({"type": "subscribe", "names": "notes.*", "body": {"title": "x"}}),
            json!("not an object"),
        ] {
            send(&mut ada, message).await;
            let error = next(&mut ada, LONG).await.unwrap();
            assert_eq!(error["code"], "kernel.ws_unknown_command", "{error}");
        }
        send(&mut ada, json!({"type": "ping"})).await;
        assert_eq!(next(&mut ada, LONG).await.unwrap(), json!({"type": "pong"}));
        // Không có gì được ghi: bảng event của module vẫn rỗng.
        let tenant = server.app.default_tenant();
        let database = tenant.database();
        let backend = sea_orm::ConnectionTrait::get_database_backend(database.read());
        let rows = sea_orm::ConnectionTrait::query_all_raw(
            database.read(),
            sea_orm::Statement::from_string(backend, "SELECT id FROM notes_events"),
        )
        .await
        .unwrap();
        assert!(rows.is_empty());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tin_hieu_chi_trong_chu_de_duoc_vao() {
    TestDatabase::run(async |test| {
        let server = server(test).await;
        let mut ada = server.connect("ada", "names=notes.*").await.unwrap();
        let mut bob = server.connect("bob", "names=notes.*").await.unwrap();
        let ada_id = Uuid::from_u128(0xada).to_string();
        let bob_id = Uuid::from_u128(0xb0b).to_string();

        send(&mut ada, json!({"type": "join", "topic": "notes.room:open"})).await;
        assert_eq!(next(&mut ada, LONG).await.unwrap(), json!({"type": "joined", "topic": "notes.room:open", "online": [ada_id]}));
        for (topic, code) in [("notes.room:closed", "kernel.ws_topic_denied"), ("x.y:1", "kernel.ws_unknown_topic")] {
            send(&mut bob, json!({"type": "join", "topic": topic})).await;
            assert_eq!(next(&mut bob, LONG).await.unwrap()["code"], code);
        }
        send(&mut bob, json!({"type": "typing", "topic": "notes.room:open"})).await;
        assert_eq!(next(&mut bob, LONG).await.unwrap()["code"], "kernel.ws_not_joined");

        send(&mut bob, json!({"type": "join", "topic": "notes.room:open"})).await;
        let joined = next(&mut bob, LONG).await.unwrap();
        assert_eq!(joined["online"].as_array().unwrap().len(), 2);
        let online = next(&mut ada, LONG).await.unwrap();
        assert_eq!(online, json!({"type": "signal", "topic": "notes.room:open", "user_id": bob_id, "signal": "online"}));
        // "Đang gõ" dày hơn 2 giây chỉ tính một lần.
        send(&mut bob, json!({"type": "typing", "topic": "notes.room:open"})).await;
        send(&mut bob, json!({"type": "typing", "topic": "notes.room:open"})).await;
        assert_eq!(next(&mut ada, LONG).await.unwrap()["signal"], "typing");
        drop(bob);
        assert_eq!(next(&mut ada, LONG).await.unwrap()["signal"], "offline");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn loi_cua_ban_tay_theo_thu_tu_nhu_sse() {
    TestDatabase::run(async |test| {
        let server = server(test).await;
        let path = "/api/v1/kernel/events/ws?names=notes.*";
        // Chưa đăng nhập: 401 trước khi xét bắt tay.
        let response = server.app.get(path).send().await.unwrap();
        assert_eq!(response.status().as_u16(), 401, "{}", response.text());
        // Tên event không có: 422, cũng trước khi xét bắt tay.
        let response = server
            .app
            .get("/api/v1/kernel/events/ws?names=notes.missing")
            .token("ada")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 422, "{}", response.text());
        // Đủ điều kiện nhưng không phải bắt tay WebSocket: 400.
        let response = server.app.get(path).token("ada").send().await.unwrap();
        assert_eq!(response.status().as_u16(), 400, "{}", response.text());
        assert_eq!(response.json()["code"], "bad_request");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn may_chu_tat_stream_tra_404_ca_khi_khong_phai_ban_tay() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test).user("ada", 0xada, &[]).build().await.unwrap();
        let response =
            app.get("/api/v1/kernel/events/ws?names=notes.*").token("ada").send().await.unwrap();
        assert_eq!(response.status().as_u16(), 404, "{}", response.text());
        assert_eq!(response.json()["code"], "kernel.stream_disabled");
    })
    .await
    .unwrap();
}
