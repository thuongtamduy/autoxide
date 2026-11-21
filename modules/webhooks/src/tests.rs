//! Test của module `webhooks` trên database gate đang chọn (gate chạy cả SQLite và
//! Postgres), với một máy nhận HTTP thật chạy trong test.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::sync::{Arc, Mutex};

use autoxide_kernel::config::{Config, ConfigReader, ConfigSource};
use autoxide_kernel::database::Database;
use autoxide_kernel::event::{Actor, Delivered, EventCatalog, EventHandler, EventInfo, Outbox};
use autoxide_kernel::job::{JobContext, JobHandler, JobQueue};
use autoxide_kernel::migration::{Migrator, ModuleMigrations};
use autoxide_kernel::module::Module;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::{HeaderMap, Method, StatusCode, header};
use sea_orm::{ConnectionTrait, EntityTrait, Statement};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::delivery::{Fanout, Sender};
use crate::entity::delivery;
use crate::service::{EndpointChanges, NewEndpoint, WebhookService};

const DEMO_EVENT: &str = "demo.thing_happened";
/// Event có `stream_permission`: chỉ người có `demo.read` được xem qua luồng event.
const PRIVATE_EVENT: &str = "demo.secret_noted";
const DEMO_READ: autoxide_kernel::access::Permission =
    autoxide_kernel::access::Permission::new("demo.read", "Read demo things.");

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
    /// Khách mặc định trên database của test.
    tenant: autoxide_kernel::tenancy::TenantContext,
    service: Arc<WebhookService>,
    fanout: Fanout,
    sender: Sender,
    /// Cài đặt theo khách mà bộ gửi đọc (`kernel.sandbox.webhook_urls`).
    settings: Arc<autoxide_testkit::FakeTenantSettings>,
}

async fn setup(test: &TestDatabase, allow_http: bool) -> Setup {
    let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
    let mut reader = ConfigReader::new(&source);
    let config = Config::read(&mut reader);
    let (config, _) = reader.finish(config).unwrap();
    let database = Database::connect(&config.database).await.unwrap();
    // Bảng của kernel (`kernel_secrets`: khoá gốc mã hoá secret ký).
    autoxide_kernel::module::Modules::new(Vec::new())
        .unwrap()
        .migrator()
        .unwrap()
        .up(&database)
        .await
        .unwrap();
    Migrator::new(vec![ModuleMigrations::new("webhooks", crate::migration::all())])
        .unwrap()
        .up(&database)
        .await
        .unwrap();
    let catalog = EventCatalog::new(vec![
        (
            "demo",
            vec![
                EventInfo::new(DEMO_EVENT, "Something happened."),
                EventInfo::new(PRIVATE_EVENT, "Something only readers see.").streamable(DEMO_READ),
            ],
        ),
        ("webhooks", crate::events::all()),
    ])
    .unwrap();
    let secrets = autoxide_kernel::secret::SecretKey::new(database.clone(), None);
    let service = Arc::new(WebhookService::new(
        secrets.clone(),
        Outbox::standalone("webhooks"),
        catalog,
        allow_http,
        // Máy nhận trong test chạy trên 127.0.0.1.
        allow_http,
    ));
    let fanout = Fanout { jobs: JobQueue::new("webhooks") };
    let client = Sender::client(std::time::Duration::from_secs(5), allow_http).unwrap();
    let settings = Arc::new(autoxide_testkit::FakeTenantSettings::default());
    let sender =
        Sender { secrets, client, allow_private_networks: allow_http, settings: settings.clone() };
    let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
    Setup { database, tenant, service, fanout, sender, settings }
}

fn event(database: &Database, name: &str) -> Delivered {
    Delivered::new(
        autoxide_kernel::tenancy::TenantContext::default_on(database.clone()),
        "demo",
        name.to_owned(),
        json!({"after": {"size": 3}}),
    )
    .actor(Actor::user(Uuid::from_u128(0xada)))
    .subject(Some("demo.thing:1".to_owned()))
}

async fn deliveries(database: &Database) -> Vec<delivery::Model> {
    delivery::Entity::find().all(database.read()).await.unwrap()
}

fn job(database: &Database, delivery_id: Uuid, attempt: i32) -> JobContext {
    JobContext::for_tenant(
        autoxide_kernel::tenancy::TenantContext::default_on(database.clone()),
        crate::delivery::SEND_KIND,
        json!({"delivery_id": delivery_id}),
        attempt,
    )
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
        let created = setup.service.create(&setup.tenant, Actor::system(), input).await.unwrap();

        // Event không ai đăng ký: không gửi. Event được giao hai lần: chỉ một lần gửi.
        setup.fanout.handle(&event(&setup.database, "webhooks.endpoint_created")).await.unwrap();
        let happened = event(&setup.database, DEMO_EVENT);
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
        let first = setup.sender.run(&job(&setup.database, rows[0].id, 1)).await;
        assert!(first.is_err());
        let row = &deliveries(&setup.database).await[0];
        assert_eq!(
            (row.status.as_str(), row.attempts, row.response_status),
            ("pending", 1, Some(500))
        );
        // Lần hai: 200 → xong.
        setup.sender.run(&job(&setup.database, rows[0].id, 2)).await.unwrap();
        let row = &deliveries(&setup.database).await[0];
        assert_eq!(
            (row.status.as_str(), row.attempts, row.response_status),
            ("succeeded", 2, Some(200))
        );
        assert!(row.delivered_at.is_some());
        // Chạy lại job đã xong: không gửi thêm.
        setup.sender.run(&job(&setup.database, rows[0].id, 3)).await.unwrap();

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
        setup.service.create(&setup.tenant, Actor::system(), input).await.unwrap();
        let fanout = Arc::new(setup.fanout);
        let happened = event(&setup.database, DEMO_EVENT);
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
        let endpoint =
            setup.service.create(&setup.tenant, Actor::system(), input).await.unwrap().endpoint;

        setup.fanout.handle(&event(&setup.database, DEMO_EVENT)).await.unwrap();
        setup.fanout.handle(&event(&setup.database, DEMO_EVENT)).await.unwrap();
        let rows = deliveries(&setup.database).await;
        assert_eq!(rows.len(), 2);

        // Lần thử cuối cùng mà vẫn lỗi: thất bại hẳn.
        let last = crate::delivery::MAX_ATTEMPTS;
        assert!(setup.sender.run(&job(&setup.database, rows[0].id, last)).await.is_err());
        let failed = delivery::Entity::find_by_id(rows[0].id).one(setup.database.read()).await;
        let failed = failed.unwrap().unwrap();
        assert_eq!(failed.status, "failed");
        assert!(failed.last_error.unwrap().contains("502"));
        // Job chạy lại sau khi hỏng (`autoxide jobs retry` đặt lại số lần thử): lần gửi được mở
        // lại và gửi thật (D-161). Bên nhận giờ trả 2xx.
        setup.sender.run(&job(&setup.database, rows[0].id, 1)).await.unwrap();
        let resent = delivery::Entity::find_by_id(rows[0].id).one(setup.database.read()).await;
        assert_eq!(resent.unwrap().unwrap().status, "succeeded");
        assert_eq!(receiver.requests.lock().unwrap().len(), 2);

        // Endpoint bị tắt: lần gửi còn chờ thất bại, không gửi đi.
        let off = EndpointChanges { disabled: Some(true), ..Default::default() };
        setup.service.update(&setup.tenant, Actor::system(), endpoint.id, off, None).await.unwrap();
        setup.sender.run(&job(&setup.database, rows[1].id, 1)).await.unwrap();
        let skipped = delivery::Entity::find_by_id(rows[1].id).one(setup.database.read()).await;
        assert_eq!(skipped.unwrap().unwrap().status, "failed");
        assert_eq!(receiver.requests.lock().unwrap().len(), 2);
        // Endpoint tắt không nhận event mới.
        setup.fanout.handle(&event(&setup.database, DEMO_EVENT)).await.unwrap();
        assert_eq!(deliveries(&setup.database).await.len(), 2);
    })
    .await
    .unwrap();
}

/// Địa chỉ trong mạng riêng (vd đã lưu trước khi tắt `ALLOW_PRIVATE_NETWORKS`): không gửi,
/// thất bại hẳn, lịch sử chỉ ghi lý do cố định (D-149).
#[tokio::test]
async fn khong_gui_vao_mang_rieng() {
    TestDatabase::run(async |test| {
        let setup = setup(test, false).await;
        let input = NewEndpoint {
            url: "https://example.com/hook".to_owned(),
            description: String::new(),
            events: vec!["*".to_owned()],
        };
        let endpoint =
            setup.service.create(&setup.tenant, Actor::system(), input).await.unwrap().endpoint;
        let backend = setup.database.write().get_database_backend();
        let sql = format!(
            "UPDATE webhooks_endpoint SET url = 'https://127.0.0.1:9/hook' WHERE id = '{}'",
            endpoint.id
        );
        let sql = if backend == sea_orm::DatabaseBackend::Sqlite {
            // SQLite lưu UUID dạng blob.
            format!(
                "UPDATE webhooks_endpoint SET url = 'https://127.0.0.1:9/hook' WHERE id = x'{}'",
                endpoint.id.simple()
            )
        } else {
            sql
        };
        setup.database.write().execute_unprepared(&sql).await.unwrap();

        setup.fanout.handle(&event(&setup.database, DEMO_EVENT)).await.unwrap();
        let rows = deliveries(&setup.database).await;
        setup.sender.run(&job(&setup.database, rows[0].id, 1)).await.unwrap();
        let row = delivery::Entity::find_by_id(rows[0].id).one(setup.database.read()).await;
        let row = row.unwrap().unwrap();
        assert_eq!(row.status, "failed");
        assert!(row.last_error.unwrap().contains("private"));
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
        let insecure = setup.service.create(
            &setup.tenant,
            Actor::system(),
            new("http://example.com/h", DEMO_EVENT),
        );
        assert_eq!(insecure.await.unwrap_err().fields()[0].field, "url");
        let unknown = setup.service.create(
            &setup.tenant,
            Actor::system(),
            new("https://example.com/h", "x.y"),
        );
        assert_eq!(unknown.await.unwrap_err().fields()[0].field, "events[0]");
        // Mọi lỗi cùng một lần.
        let all_wrong = NewEndpoint {
            url: "http://example.com/h".to_owned(),
            description: "x".repeat(201),
            events: vec!["x.y".to_owned()],
        };
        let error =
            setup.service.create(&setup.tenant, Actor::system(), all_wrong).await.unwrap_err();
        let fields: Vec<&str> = error.fields().iter().map(|field| field.field.as_str()).collect();
        assert_eq!(fields, ["url", "description", "events[0]"]);
        // Mạng riêng hay máy này: từ chối ngay (SSRF, D-149) — kể cả IP viết kiểu khác.
        for private in [
            "https://127.0.0.1/h",
            "https://localhost:8443/h",
            "https://[::1]/h",
            "https://169.254.169.254/latest/meta-data",
            "https://10.0.0.5/h",
            "https://2130706433/h",
            "https://[::ffff:192.168.1.1]/h",
        ] {
            let refused = setup
                .service
                .create(&setup.tenant, Actor::system(), new(private, DEMO_EVENT))
                .await;
            let error = refused.unwrap_err();
            assert_eq!(error.fields()[0].field, "url", "{private}");
            assert!(error.fields()[0].message.contains("private"), "{private}");
        }
        for bad in ["not a url", "https://user:pass@example.com/h", "ftp://example.com"] {
            assert!(
                setup
                    .service
                    .create(&setup.tenant, Actor::system(), new(bad, DEMO_EVENT))
                    .await
                    .is_err(),
                "{bad}"
            );
        }
        let none = NewEndpoint {
            url: "https://example.com/h".to_owned(),
            description: String::new(),
            events: vec![],
        };
        assert_eq!(
            setup.service.create(&setup.tenant, Actor::system(), none).await.unwrap_err().code(),
            "validation_failed"
        );
        let ok = setup
            .service
            .create(&setup.tenant, Actor::system(), new("https://example.com/h", DEMO_EVENT))
            .await;
        let created = ok.unwrap();
        assert!(created.secret.starts_with("whsec_"));
        let rotated = setup
            .service
            .rotate_secret(&setup.tenant, Actor::system(), created.endpoint.id)
            .await
            .unwrap();
        assert_ne!(rotated, created.secret);
    })
    .await
    .unwrap();
}

/// Module `demo` giả: chỉ khai event `demo.thing_happened`, để endpoint đăng ký nhận được nó.
struct Demo;

impl Module for Demo {
    fn name(&self) -> &'static str {
        "demo"
    }

    fn events(&self) -> Vec<EventInfo> {
        vec![EventInfo::new(DEMO_EVENT, "Something happened.")]
    }
}

#[tokio::test]
async fn quan_ly_qua_api_khong_lo_secret() {
    TestDatabase::run(async |test| {
        // `admin`: có `webhooks.manage`; `nobody`: không có permission nào.
        let app = TestApp::builder(test)
            .user("admin", 1, &["webhooks.manage"])
            .user("nobody", 1, &[])
            .module(Demo)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let call = async |method: &str, path: &str, who: &str, body: Option<Value>| {
            let method = Method::from_bytes(method.as_bytes()).unwrap();
            let mut request = app
                .request(method, &format!("/api/v1/webhooks{path}"))
                .token(who)
                .header(header::CONTENT_TYPE.as_str(), "application/json");
            if let Some(body) = body {
                request = request.bytes(body.to_string());
            }
            let answer = request.send().await.unwrap();
            let json = if answer.body().is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(answer.body()).unwrap()
            };
            (answer.status(), json)
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

/// Đề bài giao diện B3: mỗi endpoint kèm lần gửi mới nhất (không phải gọi `/deliveries` cho
/// từng endpoint), lọc endpoint theo trạng thái.
#[tokio::test]
async fn endpoint_kem_lan_gui_moi_nhat_va_loc_trang_thai() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test)
            .user("admin", 1, &["webhooks.manage"])
            .module(Demo)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let post = async |url: &str| {
            let body = json!({"url": url, "events": [DEMO_EVENT]});
            let answer =
                app.post("/api/v1/webhooks").token("admin").json(body).send().await.unwrap();
            assert_eq!(answer.status(), StatusCode::CREATED);
            answer.json()["data"]["id"].as_str().unwrap().parse::<Uuid>().unwrap()
        };
        let busy = post("https://example.com/busy").await;
        let quiet = post("https://example.com/quiet").await;
        let now = Timestamp::now();
        for (minutes, status, code) in [(10, "failed", Some(500)), (1, "succeeded", Some(204))] {
            let row = delivery::Model {
                tenant_id: app.default_tenant().id().as_uuid(),
                id: Uuid::now_v7(),
                endpoint_id: busy,
                event_id: Uuid::now_v7(),
                event_name: DEMO_EVENT.to_owned(),
                body: "{}".to_owned(),
                status: status.to_owned(),
                attempts: 1,
                response_status: code,
                last_error: None,
                created_at: now - SignedDuration::from_mins(minutes),
                last_attempt_at: Some(now),
                delivered_at: None,
            };
            delivery::Entity::insert(sea_orm::IntoActiveModel::into_active_model(row))
                .exec(app.database().write())
                .await
                .unwrap();
        }
        let disable = json!({"disabled": true});
        let path = format!("/api/v1/webhooks/{quiet}");
        let answer = app.patch(&path).token("admin").json(disable).send().await.unwrap();
        assert_eq!(answer.status(), StatusCode::OK, "{}", answer.text());

        let list = app.get("/api/v1/webhooks?order=asc").token("admin").send().await.unwrap();
        let rows = list.json()["data"].clone();
        assert_eq!(rows[0]["last_delivery"]["status"], "succeeded");
        assert_eq!(rows[0]["last_delivery"]["response_status"], 204);
        assert!(rows[1]["last_delivery"].is_null(), "{rows}");
        let one = app.get(&format!("/api/v1/webhooks/{busy}")).token("admin").send().await.unwrap();
        assert_eq!(one.json()["data"]["last_delivery"]["status"], "succeeded");

        let active = app.get("/api/v1/webhooks?status=active").token("admin").send().await.unwrap();
        let ids: Vec<_> =
            active.json()["data"].as_array().unwrap().iter().map(|r| r["id"].clone()).collect();
        assert_eq!(ids, [json!(busy)]);
        let off = app.get("/api/v1/webhooks?status=disabled").token("admin").send().await.unwrap();
        assert_eq!(off.json()["data"][0]["id"], json!(quiet));
    })
    .await
    .unwrap();
}

/// Bước 33 (D-278): event của khách `a` chỉ tạo lần gửi tới endpoint của `a` (khách `b` có
/// endpoint nghe cùng event mà không nhận gì); secret ký lưu đã mã hoá; người của `b` không đọc
/// được endpoint của `a`.
#[tokio::test]
async fn event_khach_a_khong_gui_toi_endpoint_khach_b() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test)
            .tenant("a")
            .tenant_dedicated("b")
            .user_in("a", "admin-a", 1, &["webhooks.manage"])
            .user_in("b", "admin-b", 2, &["webhooks.manage"])
            .module(Demo)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let marker = "marker-hook-a";
        let mut ids = Vec::new();
        for (who, description) in [("admin-a", marker), ("admin-b", "b")] {
            let body = json!({"url": "https://example.com/hook", "description": description, "events": [DEMO_EVENT]});
            let answer = app.post("/api/v1/webhooks").token(who).json(body).send().await.unwrap();
            assert_eq!(answer.status(), StatusCode::CREATED, "{}", answer.json());
            ids.push(answer.json()["data"]["id"].as_str().unwrap().to_owned());
        }
        let (a, b) = (app.tenant("a").await.unwrap(), app.tenant("b").await.unwrap());
        let stored = crate::entity::endpoint::Entity::find().all(a.database().read()).await.unwrap();
        let mine: Vec<_> = stored.iter().filter(|row| row.tenant_id == a.id().as_uuid()).collect();
        assert_eq!(mine.len(), 1);
        assert!(mine[0].secret.starts_with("sealed:v1:"), "the signing secret is stored encrypted");

        let event = Delivered::new(a.clone(), "demo", DEMO_EVENT.to_owned(), json!({"after": {}}));
        let fanout = Fanout { jobs: JobQueue::new("webhooks") };
        fanout.handle(&event).await.unwrap();
        let in_a = delivery::Entity::find().all(a.database().read()).await.unwrap();
        let in_a: Vec<_> = in_a.into_iter().filter(|row| row.tenant_id == a.id().as_uuid()).collect();
        assert_eq!(in_a.len(), 1);
        assert_eq!(in_a[0].endpoint_id.to_string(), ids[0]);
        assert!(delivery::Entity::find().all(b.database().read()).await.unwrap().is_empty());

        let seeded = [("id", ids[0].clone())];
        let violations = autoxide_testkit::leak_check(&app, "admin-b", marker, &seeded).await.unwrap();
        assert_eq!(violations, Vec::<String>::new());
    })
    .await
    .unwrap();
}

/// Bàn kỹ thuật (bước 52): lần gửi đã hỏng hẳn được gửi lại bằng tay (cùng thân, cùng
/// `webhook-id`); lần gửi đang chờ thì không; danh sách cả khách lọc theo trạng thái.
#[tokio::test]
async fn gui_lai_lan_giao_da_hong() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true).await;
        let receiver = Receiver::default();
        receiver.answers.lock().unwrap().push(StatusCode::BAD_GATEWAY);
        let url = receiver.start().await;
        let input = NewEndpoint { url, description: String::new(), events: vec!["*".to_owned()] };
        setup.service.create(&setup.tenant, Actor::system(), input).await.unwrap();
        setup.fanout.handle(&event(&setup.database, DEMO_EVENT)).await.unwrap();
        let id = deliveries(&setup.database).await[0].id;

        // Đang chờ: không gửi lại được.
        let busy = setup.service.redeliver(&setup.tenant, Actor::system(), id).await.unwrap_err();
        assert_eq!(busy.code(), "webhooks.delivery_in_progress");

        let last = crate::delivery::MAX_ATTEMPTS;
        assert!(setup.sender.run(&job(&setup.database, id, last)).await.is_err());
        let page = autoxide_kernel::pagination::PageRequest::try_from(
            autoxide_kernel::pagination::PageQuery::default(),
        )
        .unwrap();
        let failed =
            setup.service.all_deliveries(&setup.tenant, Some("failed"), None, &page).await.unwrap();
        assert_eq!(failed.data.len(), 1);
        assert!(
            setup
                .service
                .all_deliveries(&setup.tenant, Some("succeeded"), None, &page)
                .await
                .unwrap()
                .data
                .is_empty()
        );
        assert!(
            setup.service.all_deliveries(&setup.tenant, Some("lost"), None, &page).await.is_err()
        );

        let again = setup.service.redeliver(&setup.tenant, Actor::system(), id).await.unwrap();
        assert_eq!(again.status, "pending");
        // Job mới đã xếp: hai job (lần đầu và lần gửi lại).
        let backend = setup.database.read().get_database_backend();
        let sql = "SELECT COUNT(*) AS n FROM webhooks_jobs";
        let count = setup.database.read().query_one_raw(Statement::from_string(backend, sql));
        let jobs: i64 = count.await.unwrap().unwrap().try_get("", "n").unwrap();
        assert_eq!(jobs, 2);
        setup.sender.run(&job(&setup.database, id, 1)).await.unwrap();
        let row =
            delivery::Entity::find_by_id(id).one(setup.database.read()).await.unwrap().unwrap();
        assert_eq!(row.status, "succeeded");
        let requests = receiver.requests.lock().unwrap().clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].0["webhook-id"], requests[1].0["webhook-id"]);
        assert_eq!(requests[0].1, requests[1].1, "cùng thân");

        let missing = setup
            .service
            .redeliver(&setup.tenant, Actor::system(), Uuid::now_v7())
            .await
            .unwrap_err();
        assert_eq!(missing.code(), "webhooks.delivery_not_found");
    })
    .await
    .unwrap();
}

/// Sandbox (bước 38, D-232): webhook tới URL không có trong `kernel.sandbox.webhook_urls` không được
/// gửi — lần giao ghi `sandboxed`, nội dung vào hộp thư giả; URL được cho phép thì gửi thật.
#[tokio::test]
async fn sandbox_chi_gui_toi_url_duoc_phep() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true).await;
        let receiver = Receiver::default();
        let url = receiver.start().await;
        let input = NewEndpoint {
            url: url.clone(),
            description: "test".to_owned(),
            events: vec![DEMO_EVENT.to_owned()],
        };
        setup.service.create(&setup.tenant, Actor::system(), input).await.unwrap();
        // Ngữ cảnh sandbox của cùng khách (test dựng tay, không qua sổ khách).
        let sandbox = setup
            .tenant
            .clone()
            .with_sandbox_of(autoxide_kernel::tenancy::TenantId::from_uuid(Uuid::now_v7()), None);
        let send = |id: Uuid| {
            JobContext::for_tenant(
                sandbox.clone(),
                crate::delivery::SEND_KIND,
                json!({"delivery_id": id}),
                1,
            )
        };

        setup.fanout.handle(&event(&setup.database, DEMO_EVENT)).await.unwrap();
        let first = deliveries(&setup.database).await[0].id;
        setup.sender.run(&send(first)).await.unwrap();
        let row = delivery::Entity::find_by_id(first).one(setup.database.read()).await.unwrap();
        assert_eq!(row.unwrap().status, "sandboxed");
        assert!(receiver.requests.lock().unwrap().is_empty(), "không gửi ra ngoài");
        let backend = setup.database.read().get_database_backend();
        let sql = "SELECT channel, reason, recipient FROM kernel_sandbox_messages";
        let held = setup
            .database
            .read()
            .query_all_raw(Statement::from_string(backend, sql))
            .await
            .unwrap();
        assert_eq!(held.len(), 1);
        let channel: String = held[0].try_get("", "channel").unwrap();
        let reason: String = held[0].try_get("", "reason").unwrap();
        let recipient: String = held[0].try_get("", "recipient").unwrap();
        assert_eq!((channel.as_str(), reason.as_str()), ("webhook", "url_not_allowed"));
        assert_eq!(recipient, url);

        // Cho phép tiền tố của máy nhận: lần giao sau đi thật.
        let prefix = url.trim_end_matches("/hook");
        setup.settings.set(setup.tenant.id(), "kernel.sandbox.webhook_urls", prefix);
        let mut second = event(&setup.database, DEMO_EVENT);
        second.id = Uuid::now_v7();
        setup.fanout.handle(&second).await.unwrap();
        let next =
            deliveries(&setup.database).await.into_iter().find(|row| row.id != first).unwrap().id;
        setup.sender.run(&send(next)).await.unwrap();
        let row = delivery::Entity::find_by_id(next).one(setup.database.read()).await.unwrap();
        assert_eq!(row.unwrap().status, "succeeded");
        assert_eq!(receiver.requests.lock().unwrap().len(), 1);
    })
    .await
    .unwrap();
}

/// Đăng ký webhook cũng đòi `stream_permission` của event như luồng event (SSE / WebSocket).
#[tokio::test]
async fn dang_ky_event_can_quyen_xem_event() {
    TestDatabase::run(async |test| {
        let setup = setup(test, false).await;
        let service = &setup.service;
        let root = autoxide_kernel::tenancy::root_of(autoxide_kernel::tenancy::DEFAULT_TENANT);
        let person = |permissions: &[&str]| {
            autoxide_kernel::access::Principal::with_grants(
                Uuid::from_u128(7),
                permissions.iter().map(|permission| {
                    (
                        (*permission).to_owned(),
                        autoxide_kernel::access::Grant::new(
                            root,
                            autoxide_kernel::access::Scope::Tenant,
                        ),
                    )
                }),
            )
        };
        let manager = person(&["webhooks.manage"]);
        let reader = person(&["webhooks.manage", "demo.read"]);
        for events in [vec![PRIVATE_EVENT.to_owned()], vec!["*".to_owned()]] {
            let error = service.require_event_permissions(&manager, &events).unwrap_err();
            assert_eq!(error.code(), "permission_denied", "{events:?}");
            service.require_event_permissions(&reader, &events).unwrap();
        }
        // Event không có `stream_permission`: như trước.
        service.require_event_permissions(&manager, &[DEMO_EVENT.to_owned()]).unwrap();
    })
    .await
    .unwrap();
}
