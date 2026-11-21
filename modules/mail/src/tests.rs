//! Test của module `mail` trên database gate đang chọn: mail vào job queue, job gửi qua
//! file hoặc qua một máy chủ SMTP giả chạy ngay trong test.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use autoxide_kernel::config::{Config, ConfigReader, ConfigSource};
use autoxide_kernel::database::Database;
use autoxide_kernel::mail::{Mail, Mailer};
use autoxide_kernel::module::Modules;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_testkit::TestDatabase;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

/// Thư mục tạm riêng cho một test, tự xoá khi xong.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("autoxide-mail-{label}-{}", uuid::Uuid::now_v7()));
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Module `mail` đã cấu hình bằng `pairs`, migrate và khởi động; trả về `Mailer` và
/// worker chạy job gửi mail.
async fn setup(
    test: &TestDatabase,
    pairs: &[(&str, &str)],
) -> (Arc<dyn Mailer>, autoxide_kernel::job::Worker, TenantContext) {
    let (mailer, worker, database) = setup_with_database(test, pairs).await;
    (mailer, worker, autoxide_kernel::tenancy::DatabaseRegistry::single(database).default_tenant())
}

async fn setup_with_database(
    test: &TestDatabase,
    pairs: &[(&str, &str)],
) -> (Arc<dyn Mailer>, autoxide_kernel::job::Worker, Database) {
    let mut all = vec![("DATABASE_URL", test.url())];
    all.extend_from_slice(pairs);
    let source = ConfigSource::from_pairs(all);
    let mut modules = Modules::new(vec![Box::new(crate::module())]).unwrap();
    let mut reader = ConfigReader::new(&source);
    let config = Config::read(&mut reader);
    let configured = modules.configure(&mut reader);
    configured.unwrap();
    let (config, _) = reader.finish(config).unwrap();
    let database = Database::connect(&config.database).await.unwrap();
    modules.migrator().unwrap().up(&database).await.unwrap();
    modules.start(&database).unwrap();
    let mailer = modules.api::<dyn Mailer>().unwrap();
    let worker = modules.worker(4).unwrap();
    (mailer, worker, database)
}

fn welcome(to: &str) -> Mail {
    Mail::new(to, "Welcome", "Hello Ada, your account is ready.")
        .html("<p>Hello Ada, your account is ready.</p>".to_owned())
}

#[tokio::test]
async fn chua_cau_hinh_smtp_thi_ghi_file_eml() {
    TestDatabase::run(async |test| {
        let directory = TempDir::new("file");
        let path = directory.0.to_string_lossy().into_owned();
        let (mailer, worker, tenant) =
            setup(test, &[("AUTOXIDE_MAIL_FILE_DIR", path.as_str())]).await;

        mailer.send(&tenant, welcome("Ada <ada@example.com>")).await.unwrap();
        assert_eq!(worker.run_once().await.unwrap(), 1);

        let files: Vec<_> = std::fs::read_dir(&directory.0).unwrap().collect();
        assert_eq!(files.len(), 1);
        let content = std::fs::read_to_string(files[0].as_ref().unwrap().path()).unwrap();
        assert!(content.contains("To: Ada <ada@example.com>"), "{content}");
        assert!(content.contains("From: Autoxide <noreply@localhost>"), "{content}");
        assert!(content.contains("Subject: Welcome"), "{content}");
        assert!(content.contains("Hello Ada, your account is ready."), "{content}");
        assert!(content.contains("multipart/alternative"), "{content}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dia_chi_sai_bi_tu_choi_ngay() {
    TestDatabase::run(async |test| {
        let directory = TempDir::new("invalid");
        let path = directory.0.to_string_lossy().into_owned();
        let (mailer, _, tenant) = setup(test, &[("AUTOXIDE_MAIL_FILE_DIR", path.as_str())]).await;

        let error = mailer.send(&tenant, welcome("not an address")).await.unwrap_err();

        assert_eq!(error.code(), "validation_failed");
        assert_eq!(error.fields()[0].field, "to");
    })
    .await
    .unwrap();
}

/// Máy chủ SMTP giả: trả lời đủ để gửi một mail, ghi lại mọi dòng nhận được.
async fn fake_smtp() -> (u16, Arc<Mutex<Vec<String>>>) {
    fake_smtp_answering(b"250 ok\r\n").await
}

/// Như [`fake_smtp`], nhưng trả lời `rcpt` cho lệnh `RCPT TO` (vd từ chối người nhận).
async fn fake_smtp_answering(rcpt: &'static [u8]) -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let received = Arc::new(Mutex::new(Vec::new()));
    let log = received.clone();
    tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let (reader, mut writer) = socket.into_split();
        let mut lines = BufReader::new(reader).lines();
        writer.write_all(b"220 fake ESMTP\r\n").await.unwrap();
        let mut in_data = false;
        while let Ok(Some(line)) = lines.next_line().await {
            log.lock().unwrap().push(line.clone());
            let reply: &[u8] = if in_data {
                if line == "." {
                    in_data = false;
                    b"250 queued\r\n"
                } else {
                    continue;
                }
            } else if line.starts_with("EHLO") {
                b"250 fake\r\n"
            } else if line == "DATA" {
                in_data = true;
                b"354 go ahead\r\n"
            } else if line.starts_with("RCPT TO") {
                rcpt
            } else if line == "QUIT" {
                writer.write_all(b"221 bye\r\n").await.unwrap();
                break;
            } else {
                b"250 ok\r\n"
            };
            writer.write_all(reply).await.unwrap();
        }
    });
    (port, received)
}

#[tokio::test]
async fn co_smtp_thi_gui_qua_smtp() {
    TestDatabase::run(async |test| {
        let (port, received) = fake_smtp().await;
        // Không mã hoá: chỉ dùng cho máy chủ giả trong test.
        let url = format!("smtp://127.0.0.1:{port}");
        let (mailer, worker, tenant) =
            setup(test, &[("AUTOXIDE_MAIL_SMTP_URL", url.as_str())]).await;

        mailer.send(&tenant, welcome("ada@example.com")).await.unwrap();
        assert_eq!(worker.run_once().await.unwrap(), 1);

        let lines = received.lock().unwrap().clone();
        assert!(lines.iter().any(|line| line == "MAIL FROM:<noreply@localhost>"), "{lines:?}");
        assert!(lines.iter().any(|line| line == "RCPT TO:<ada@example.com>"), "{lines:?}");
        assert!(lines.iter().any(|line| line == "Subject: Welcome"), "{lines:?}");
        let done =
            worker.list("mail", autoxide_kernel::job::JobStatus::Completed, 10).await.unwrap();
        assert_eq!(done.len(), 1);
    })
    .await
    .unwrap();
}

/// `Reply-To` và header riêng đi theo mail; header lạ hay có ký tự xuống dòng bị từ chối khi
/// xếp hàng; mail cùng `dedupe_key` chỉ gửi một lần.
#[tokio::test]
async fn reply_to_header_rieng_va_chong_trung() {
    TestDatabase::run(async |test| {
        let directory = TempDir::new("headers");
        let path = directory.0.to_string_lossy().into_owned();
        let (mailer, worker, tenant) =
            setup(test, &[("AUTOXIDE_MAIL_FILE_DIR", path.as_str())]).await;
        let mut mail = welcome("ada@example.com");
        mail.reply_to = Some("Bob <bob@example.com>".to_owned());
        mail.headers = vec![
            ("List-Unsubscribe".to_owned(), "<https://api.example.com/u/abc>".to_owned()),
            ("List-Unsubscribe-Post".to_owned(), "List-Unsubscribe=One-Click".to_owned()),
        ];
        mail.dedupe_key = Some("delivery-1".to_owned());
        mailer.send(&tenant, mail.clone()).await.unwrap();
        mailer.send(&tenant, mail.clone()).await.unwrap();
        assert_eq!(worker.run_once().await.unwrap(), 1, "the second one is a duplicate");
        let files: Vec<_> = std::fs::read_dir(&directory.0).unwrap().collect();
        assert_eq!(files.len(), 1);
        let content = std::fs::read_to_string(files[0].as_ref().unwrap().path()).unwrap();
        assert!(content.contains("Reply-To: Bob <bob@example.com>"), "{content}");
        assert!(content.contains("List-Unsubscribe: <https://api.example.com/u/abc>"), "{content}");
        assert!(content.contains("List-Unsubscribe-Post: List-Unsubscribe=One-Click"), "{content}");

        let mut bad = welcome("ada@example.com");
        bad.reply_to = Some("not an address".to_owned());
        bad.headers = vec![
            ("Bcc".to_owned(), "eve@example.com".to_owned()),
            ("List-Id".to_owned(), "x\r\nBcc: eve@example.com".to_owned()),
        ];
        let error = mailer.send(&tenant, bad).await.unwrap_err();
        let fields: Vec<&str> = error.fields().iter().map(|field| field.field.as_str()).collect();
        assert_eq!(fields, ["reply_to", "headers[0].name", "headers[1].value"]);
    })
    .await
    .unwrap();
}

async fn failures(database: &Database) -> Vec<String> {
    use sea_orm::{ConnectionTrait, Statement};
    let backend = database.read().get_database_backend();
    let sql = "SELECT payload FROM mail_events WHERE name = 'mail.delivery_failed'";
    let rows = database.read().query_all_raw(Statement::from_string(backend, sql)).await.unwrap();
    rows.iter().map(|row| row.try_get::<String>("", "payload").unwrap()).collect()
}

/// Máy chủ nhận từ chối hẳn (550): không thử lại, phát event kèm địa chỉ và khoá; từ chối tạm
/// (450): job được thử lại.
#[tokio::test]
async fn tu_choi_han_thi_phat_event_tu_choi_tam_thi_thu_lai() {
    TestDatabase::run(async |test| {
        let (port, _) = fake_smtp_answering(b"550 5.1.1 no such user\r\n").await;
        let url = format!("smtp://127.0.0.1:{port}");
        let (mailer, worker, database) =
            setup_with_database(test, &[("AUTOXIDE_MAIL_SMTP_URL", url.as_str())]).await;
        let mut mail = welcome("ghost@example.com");
        mail.dedupe_key = Some("delivery-9".to_owned());
        mailer
            .send(
                &autoxide_kernel::tenancy::DatabaseRegistry::single(database.clone())
                    .default_tenant(),
                mail,
            )
            .await
            .unwrap();
        assert_eq!(worker.run_once().await.unwrap(), 1);
        let done =
            worker.list("mail", autoxide_kernel::job::JobStatus::Completed, 10).await.unwrap();
        assert_eq!(done.len(), 1, "not retried");
        let events = failures(&database).await;
        assert_eq!(events.len(), 1);
        let payload: serde_json::Value = serde_json::from_str(&events[0]).unwrap();
        assert_eq!(
            (payload["to"].as_str(), payload["dedupe_key"].as_str(), payload["code"].as_str()),
            (Some("ghost@example.com"), Some("delivery-9"), Some("550"))
        );
    })
    .await
    .unwrap();
    TestDatabase::run(async |test| {
        let (port, _) = fake_smtp_answering(b"450 4.2.0 try later\r\n").await;
        let url = format!("smtp://127.0.0.1:{port}");
        let (mailer, worker, database) =
            setup_with_database(test, &[("AUTOXIDE_MAIL_SMTP_URL", url.as_str())]).await;
        mailer
            .send(
                &autoxide_kernel::tenancy::DatabaseRegistry::single(database.clone())
                    .default_tenant(),
                welcome("ada@example.com"),
            )
            .await
            .unwrap();
        assert_eq!(worker.run_once().await.unwrap(), 1);
        let done =
            worker.list("mail", autoxide_kernel::job::JobStatus::Completed, 10).await.unwrap();
        assert!(done.is_empty(), "will be retried");
        assert!(failures(&database).await.is_empty());
    })
    .await
    .unwrap();
}

/// Bước 33 (D-274): khách đã đặt máy chủ SMTP và người gửi riêng thì mail của khách đi qua đó;
/// khách khác vẫn dùng cấu hình chung (ở đây: ghi file).
#[tokio::test]
async fn smtp_va_nguoi_gui_rieng_cua_khach() {
    TestDatabase::run(async |test| {
        use axum::http::{Method, StatusCode};
        let directory = TempDir::new("tenant");
        let path = directory.0.to_string_lossy().into_owned();
        let (port, received) = fake_smtp().await;
        let app = autoxide_testkit::TestApp::builder(test)
            .setting("AUTOXIDE_MAIL_FILE_DIR", &path)
            // Máy chủ SMTP giả chạy ở 127.0.0.1: người vận hành cho phép mạng riêng.
            .setting("AUTOXIDE_MAIL_ALLOW_PRIVATE_NETWORKS", "true")
            .tenant("shop")
            .user_in("shop", "owner", 1, &["kernel.settings.manage"])
            .module(crate::module())
            .build()
            .await
            .unwrap();
        // Tên theo quy ước kết nối đối tác; tên cũ (`mail.from`) vẫn được nhận.
        for (name, value) in [
            ("mail.connection.smtp.url", format!("smtp://127.0.0.1:{port}")),
            ("mail.from", "Shop <hello@shop.example>".to_owned()),
        ] {
            let answer = app
                .request(Method::PUT, &format!("/api/v1/kernel/settings/{name}"))
                .token("owner")
                .json(serde_json::json!({"value": value}))
                .send()
                .await
                .unwrap();
            assert_eq!(answer.status(), StatusCode::OK, "{}", answer.json());
            assert!(!answer.text().contains("127.0.0.1"), "a secret is never answered");
        }
        // Màn "Kết nối đối tác": cả hai nằm trong nhóm `connection`, dưới tên mới.
        let listed = app
            .get("/api/v1/kernel/settings?group=connection")
            .token("owner")
            .send()
            .await
            .unwrap()
            .json();
        let listed: Vec<(&str, bool, bool)> = listed["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| (s["name"].as_str().unwrap(), s["secret"] == true, s["is_set"] == true))
            .collect();
        assert_eq!(
            listed,
            [("mail.connection.smtp.from", false, true), ("mail.connection.smtp.url", true, true)]
        );
        let mailer = app.modules().api::<dyn Mailer>().unwrap();
        let shop = app.tenant("shop").await.unwrap();
        mailer.send(&shop, welcome("ada@example.com")).await.unwrap();
        mailer.send(&app.default_tenant(), welcome("bob@example.com")).await.unwrap();
        let worker = app.modules().worker(4).unwrap();
        while worker.run_once().await.unwrap() > 0 {}

        let lines = received.lock().unwrap().clone();
        assert!(
            lines.iter().any(|line| line.contains("MAIL FROM:<hello@shop.example>")),
            "{lines:?}"
        );
        assert!(lines.iter().any(|line| line.contains("RCPT TO:<ada@example.com>")), "{lines:?}");
        assert!(!lines.iter().any(|line| line.contains("bob@example.com")), "{lines:?}");
        let files: Vec<_> = std::fs::read_dir(&directory.0).unwrap().collect();
        assert_eq!(files.len(), 1, "the default tenant still writes files");

        // Lần dùng của kết nối (bước 51): hiện ở cài đặt, thử được từ trang cài đặt.
        let after = app
            .get("/api/v1/kernel/settings?group=connection")
            .token("owner")
            .header("autoxide-tenant", "shop")
            .send()
            .await
            .unwrap()
            .json();
        let url = after["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == "mail.connection.smtp.url")
            .cloned()
            .unwrap();
        assert!(url["last_used_at"].is_string(), "{after}");
        assert!(url.get("last_error").is_none(), "{after}");
        let tested = app
            .post("/api/v1/kernel/settings/mail.connection.smtp.url/test")
            .token("owner")
            .header("autoxide-tenant", "shop")
            .send()
            .await
            .unwrap();
        assert_eq!(tested.status(), StatusCode::OK, "{}", tested.json());
        assert_eq!(tested.json()["data"]["connection"], "mail.connection.smtp");
        let untestable = app
            .post("/api/v1/kernel/settings/kernel.public_url/test")
            .token("owner")
            .header("autoxide-tenant", "shop")
            .send()
            .await
            .unwrap();
        assert_eq!(untestable.status(), StatusCode::NOT_FOUND);
    })
    .await
    .unwrap();
}
