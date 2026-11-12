//! Test của module `webhooks` trên database gate đang chọn (gate chạy cả SQLite và
//! Postgres), với một máy nhận HTTP thật chạy trong test.

use std::sync::{Arc, Mutex};

use autoxide_kernel::access::{Authenticator, Principal};
use autoxide_kernel::config::{Config, ConfigReader, ConfigSource};
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Delivered, EventCatalog, EventHandler, EventInfo, Outbox};
use autoxide_kernel::job::{JobContext, JobHandler, JobQueue};
use autoxide_kernel::migration::{Migrator, ModuleMigrations};
use autoxide_testkit::TestDatabase;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode, header};
use sea_orm::{ConnectionTrait, EntityTrait, Statement};
use serde_json::{Value, json};
use tower::ServiceExt as _;
use uuid::Uuid;

use crate::delivery::{Fanout, Sender};
use crate::entity::delivery;
use crate::service::{EndpointChanges, NewEndpoint, WebhookService};

const DEMO_EVENT: &str = "demo.thing_happened";

/// Máy nhận: ghi lại mọi request, trả lần lượt các mã trong `answers` (hết thì 200).
#[derive(Clone, Default)]
struct Receiver {
    requests: Arc<Mutex<Vec<(HeaderMap, String)>>>,
    answers: Arc<Mutex<Vec<StatusCode>>>,
}

impl Receiver {
    /// Máy nhận HTTP tối giản trên TCP (mỗi kết nối một request, rồi đóng): không dùng
    /// router của axum vì nó không thuộc API của Autoxide.
    async fn start(&self) -> String {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/hook", listener.local_addr().unwrap());
        let receiver = self.clone();
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut raw = Vec::new();
                let mut buffer = [0u8; 4096];
                // Đọc tới hết header rồi đủ `content-length` byte thân.
                let (headers, body) = loop {
                    let read = socket.read(&mut buffer).await.unwrap();
                    raw.extend_from_slice(&buffer[..read]);
                    let text = String::from_utf8_lossy(&raw).into_owned();
                    let Some((head, body)) = text.split_once("\r\n\r\n") else { continue };
                    let mut headers = HeaderMap::new();
                    for line in head.lines().skip(1) {
                        let (name, value) = line.split_once(':').unwrap();
                        headers.insert(
                            header::HeaderName::from_bytes(name.trim().as_bytes()).unwrap(),
                            value.trim().parse().unwrap(),
                        );
                    }
                    let length: usize =
                        headers[header::CONTENT_LENGTH].to_str().unwrap().parse().unwrap();
                    if body.len() >= length || read == 0 {
                        break (headers, body.to_owned());
                    }
                };
                receiver.requests.lock().unwrap().push((headers, body));
                let status = {
                    let mut answers = receiver.answers.lock().unwrap();
                    if answers.is_empty() { StatusCode::OK } else { answers.remove(0) }
                };
                let answer = format!(
                    "HTTP/1.1 {} X\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                    status.as_u16()
                );
                socket.write_all(answer.as_bytes()).await.unwrap();
                socket.shutdown().await.unwrap();
            }
        });
        url
    }
}

struct Setup {
    database: Database,
    service: Arc<WebhookService>,
    fanout: Fanout,
    sender: Sender,
}

async fn setup(test: &TestDatabase, allow_http: bool) -> Setup {
    let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
    let mut reader = ConfigReader::new(&source);
    let config = Config::read(&mut reader);
    let (config, _) = reader.finish(config).unwrap();
    let database = Database::connect(&config.database).await.unwrap();
    Migrator::new(vec![ModuleMigrations::new("webhooks", crate::migration::all())])
        .unwrap()
        .up(&database)
        .await
        .unwrap();
    let catalog = EventCatalog::new(vec![
        ("demo", vec![EventInfo::new(DEMO_EVENT, "Something happened.")]),
        ("webhooks", crate::events::all()),
    ])
    .unwrap();
    let service = Arc::new(WebhookService::new(
        database.clone(),
        Outbox::standalone("webhooks"),
        catalog,
        allow_http,
    ));
    let fanout = Fanout { database: database.clone(), jobs: JobQueue::new("webhooks") };
    let client = Sender::client(std::time::Duration::from_secs(5)).unwrap();
    let sender = Sender { database: database.clone(), client };
    Setup { database, service, fanout, sender }
}

fn event(name: &str) -> Delivered {
    Delivered {
        id: Uuid::now_v7(),
        module: "demo",
        name: name.to_owned(),
        actor: Actor::user(Uuid::from_u128(0xada)),
        subject: Some("demo.thing:1".to_owned()),
        payload: json!({"after": {"size": 3}}),
        occurred_at: chrono::Utc::now(),
    }
}

async fn deliveries(database: &Database) -> Vec<delivery::Model> {
    delivery::Entity::find().all(database.read()).await.unwrap()
}

fn job(delivery_id: Uuid, attempt: i32) -> JobContext {
    JobContext {
        id: Uuid::now_v7(),
        kind: crate::delivery::SEND_KIND.to_owned(),
        payload: json!({"delivery_id": delivery_id}),
        attempt,
    }
}

#[tokio::test]
async fn gui_co_chu_ky_thu_lai_khi_loi_va_khong_gui_trung() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true).await;
        let receiver = Receiver::default();
        receiver.answers.lock().unwrap().push(StatusCode::INTERNAL_SERVER_ERROR);
        let url = receiver.start().await;
        let input = NewEndpoint {
            url,
            description: "test".to_owned(),
            events: vec![DEMO_EVENT.to_owned()],
        };
        let created = setup.service.create(Actor::system(), input).await.unwrap();

        // Event không ai đăng ký: không gửi. Event được giao hai lần: chỉ một lần gửi.
        setup.fanout.handle(&event("webhooks.endpoint_created")).await.unwrap();
        let happened = event(DEMO_EVENT);
        setup.fanout.handle(&happened).await.unwrap();
        setup.fanout.handle(&happened).await.unwrap();
        let rows = deliveries(&setup.database).await;
        assert_eq!(rows.len(), 1);
        let backend = setup.database.read().get_database_backend();
        let sql = "SELECT COUNT(*) AS n FROM webhooks_jobs";
        let count = setup.database.read().query_one_raw(Statement::from_string(backend, sql));
        let jobs: i64 = count.await.unwrap().unwrap().try_get("", "n").unwrap();
        assert_eq!(jobs, 1);

        // Lần một: bên nhận trả 500 → job lỗi (để thử lại), lần gửi còn chờ.
        let first = setup.sender.run(&job(rows[0].id, 1)).await;
        assert!(first.is_err());
        let row = &deliveries(&setup.database).await[0];
        assert_eq!(
            (row.status.as_str(), row.attempts, row.response_status),
            ("pending", 1, Some(500))
        );
        // Lần hai: 200 → xong.
        setup.sender.run(&job(rows[0].id, 2)).await.unwrap();
        let row = &deliveries(&setup.database).await[0];
        assert_eq!(
            (row.status.as_str(), row.attempts, row.response_status),
            ("succeeded", 2, Some(200))
        );
        assert!(row.delivered_at.is_some());
        // Chạy lại job đã xong: không gửi thêm.
        setup.sender.run(&job(rows[0].id, 3)).await.unwrap();

        let requests = receiver.requests.lock().unwrap().clone();
        assert_eq!(requests.len(), 2);
        let (headers, body) = &requests[1];
        let id = headers["webhook-id"].to_str().unwrap();
        assert_eq!(id, rows[0].id.to_string());
        assert_eq!(id, requests[0].0["webhook-id"].to_str().unwrap());
        let timestamp: i64 = headers["webhook-timestamp"].to_str().unwrap().parse().unwrap();
        let expected = crate::signature::sign(&created.secret, id, timestamp, body).unwrap();
        assert_eq!(headers["webhook-signature"].to_str().unwrap(), expected);
        assert_eq!(headers[header::CONTENT_TYPE], "application/json");
        let body: Value = serde_json::from_str(body).unwrap();
        assert_eq!(body["type"], DEMO_EVENT);
        assert_eq!(body["data"]["id"], happened.id.to_string());
        assert_eq!(body["data"]["payload"]["after"]["size"], 3);
    })
    .await
    .unwrap();
}

/// Cùng một event được giao song song nhiều lần (nhiều bộ phát): chỉ một lần gửi và
/// một job — trên cả hai database.
#[tokio::test]
async fn giao_dong_thoi_khong_gui_trung() {
    TestDatabase::run(async |test| {
        let setup = setup(test, false).await;
        let input = NewEndpoint {
            url: "https://example.com/hook".to_owned(),
            description: String::new(),
            events: vec![DEMO_EVENT.to_owned()],
        };
        setup.service.create(Actor::system(), input).await.unwrap();
        let fanout = Arc::new(setup.fanout);
        let happened = event(DEMO_EVENT);
        let handles: Vec<_> = (0..6)
            .map(|_| {
                let (fanout, happened) = (fanout.clone(), happened.clone());
                tokio::spawn(async move { fanout.handle(&happened).await })
            })
            .collect();
        for handle in handles {
            handle.await.unwrap().unwrap();
        }
        assert_eq!(deliveries(&setup.database).await.len(), 1);
        let backend = setup.database.read().get_database_backend();
        let sql = "SELECT COUNT(*) AS n FROM webhooks_jobs";
        let count = setup.database.read().query_one_raw(Statement::from_string(backend, sql));
        let jobs: i64 = count.await.unwrap().unwrap().try_get("", "n").unwrap();
        assert_eq!(jobs, 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn het_lan_thu_hoac_endpoint_tat_thi_that_bai() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true).await;
        let receiver = Receiver::default();
        receiver.answers.lock().unwrap().push(StatusCode::BAD_GATEWAY);
        let url = receiver.start().await;
        let input = NewEndpoint { url, description: String::new(), events: vec!["*".to_owned()] };
        let endpoint = setup.service.create(Actor::system(), input).await.unwrap().endpoint;

        setup.fanout.handle(&event(DEMO_EVENT)).await.unwrap();
        setup.fanout.handle(&event(DEMO_EVENT)).await.unwrap();
        let rows = deliveries(&setup.database).await;
        assert_eq!(rows.len(), 2);

        // Lần thử cuối cùng mà vẫn lỗi: thất bại hẳn.
        let last = crate::delivery::MAX_ATTEMPTS;
        assert!(setup.sender.run(&job(rows[0].id, last)).await.is_err());
        let failed = delivery::Entity::find_by_id(rows[0].id).one(setup.database.read()).await;
        let failed = failed.unwrap().unwrap();
        assert_eq!(failed.status, "failed");
        assert!(failed.last_error.unwrap().contains("502"));

        // Endpoint bị tắt: lần gửi còn chờ thất bại, không gửi đi.
        let off = EndpointChanges { disabled: Some(true), ..Default::default() };
        setup.service.update(Actor::system(), endpoint.id, off).await.unwrap();
        setup.sender.run(&job(rows[1].id, 1)).await.unwrap();
        let skipped = delivery::Entity::find_by_id(rows[1].id).one(setup.database.read()).await;
        assert_eq!(skipped.unwrap().unwrap().status, "failed");
        assert_eq!(receiver.requests.lock().unwrap().len(), 1);
        // Endpoint tắt không nhận event mới.
        setup.fanout.handle(&event(DEMO_EVENT)).await.unwrap();
        assert_eq!(deliveries(&setup.database).await.len(), 2);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn kiem_dia_chi_va_ten_event() {
    TestDatabase::run(async |test| {
        let setup = setup(test, false).await;
        let new = |url: &str, event: &str| NewEndpoint {
            url: url.to_owned(),
            description: String::new(),
            events: vec![event.to_owned()],
        };
        let insecure =
            setup.service.create(Actor::system(), new("http://example.com/h", DEMO_EVENT));
        assert_eq!(insecure.await.unwrap_err().code(), "webhooks.insecure_url");
        let unknown = setup.service.create(Actor::system(), new("https://example.com/h", "x.y"));
        assert_eq!(unknown.await.unwrap_err().code(), "webhooks.unknown_event");
        for bad in ["not a url", "https://user:pass@example.com/h", "ftp://example.com"] {
            assert!(
                setup.service.create(Actor::system(), new(bad, DEMO_EVENT)).await.is_err(),
                "{bad}"
            );
        }
        let none = NewEndpoint {
            url: "https://example.com/h".to_owned(),
            description: String::new(),
            events: vec![],
        };
        assert_eq!(
            setup.service.create(Actor::system(), none).await.unwrap_err().code(),
            "validation_failed"
        );
        let ok =
            setup.service.create(Actor::system(), new("https://example.com/h", DEMO_EVENT)).await;
        let created = ok.unwrap();
        assert!(created.secret.starts_with("whsec_"));
        let rotated =
            setup.service.rotate_secret(Actor::system(), created.endpoint.id).await.unwrap();
        assert_ne!(rotated, created.secret);
    })
    .await
    .unwrap();
}

/// `Bearer admin`: có `webhooks.manage`; còn lại: không có permission nào.
struct FakeAuthenticator;

#[async_trait::async_trait]
impl Authenticator for FakeAuthenticator {
    async fn authenticate(&self, authorization: Option<&str>) -> Result<Principal, Error> {
        let id = Uuid::from_u128(1);
        Ok(match authorization {
            Some("Bearer admin") => Principal::new(id, ["webhooks.manage".to_owned()]),
            _ => Principal::new(id, []),
        })
    }
}

#[tokio::test]
async fn quan_ly_qua_api_khong_lo_secret() {
    TestDatabase::run(async |test| {
        let setup = setup(test, false).await;
        let source = ConfigSource::default();
        let mut reader = ConfigReader::new(&source);
        let config = Config::read(&mut reader);
        let (config, _) = reader.finish(config).unwrap();
        let app = autoxide_kernel::http::build(
            &config.http,
            utoipa_axum::router::OpenApiRouter::new(),
            vec![("webhooks", crate::http::routes(setup.service.clone()))],
            autoxide_kernel::http::Extras {
                authenticator: Some(Arc::new(FakeAuthenticator)),
                ..Default::default()
            },
        )
        .router;
        let call = async |method: &str, path: &str, who: &str, body: Option<Value>| {
            let request = Request::builder()
                .method(method)
                .uri(format!("/api/v1/webhooks{path}"))
                .header(header::AUTHORIZATION, format!("Bearer {who}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
                .unwrap();
            let response = app.clone().oneshot(request).await.unwrap();
            let status = response.status();
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let json = if bytes.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&bytes).unwrap()
            };
            (status, json)
        };

        let (status, _) = call("GET", "", "nobody", None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, events) = call("GET", "/events", "admin", None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(events["data"].as_array().unwrap().iter().any(|event| event["name"] == DEMO_EVENT));

        let body = json!({"url": "https://example.com/h", "events": [DEMO_EVENT]});
        let (status, created) = call("POST", "", "admin", Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        assert!(created["data"]["secret"].as_str().unwrap().starts_with("whsec_"));
        let id = created["data"]["id"].as_str().unwrap().to_owned();

        let (status, list) = call("GET", "", "admin", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(list["data"][0]["id"], id.as_str());
        assert!(list["data"][0].get("secret").is_none());
        let (status, changed) =
            call("PATCH", &format!("/{id}"), "admin", Some(json!({"disabled": true}))).await;
        assert_eq!(status, StatusCode::OK);
        assert!(changed["data"]["disabled_at"].is_string());
        let (status, deliveries) = call("GET", &format!("/{id}/deliveries"), "admin", None).await;
        assert_eq!((status, deliveries["data"].clone()), (StatusCode::OK, json!([])));
        let (status, secret) = call("POST", &format!("/{id}/secret"), "admin", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_ne!(secret["data"]["secret"], created["data"]["secret"]);
        let (status, _) = call("DELETE", &format!("/{id}"), "admin", None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, missing) = call("GET", &format!("/{id}"), "admin", None).await;
        assert_eq!(
            (status, missing["code"].clone()),
            (StatusCode::NOT_FOUND, json!("webhooks.endpoint_not_found"))
        );
    })
    .await
    .unwrap();
}
