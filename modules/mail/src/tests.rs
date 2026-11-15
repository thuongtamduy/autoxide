//! Test của module `mail` trên database gate đang chọn: mail vào job queue, job gửi qua
//! file hoặc qua một máy chủ SMTP giả chạy ngay trong test.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use autoxide_kernel::config::{Config, ConfigReader, ConfigSource};
use autoxide_kernel::database::Database;
use autoxide_kernel::mail::{Mail, Mailer};
use autoxide_kernel::module::Modules;
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
) -> (Arc<dyn Mailer>, autoxide_kernel::job::Worker) {
    let (mailer, worker, _) = setup_with_database(test, pairs).await;
    (mailer, worker)
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
        let (mailer, worker) = setup(test, &[("AUTOXIDE_MAIL_FILE_DIR", path.as_str())]).await;

        mailer.send(welcome("Ada <ada@example.com>")).await.unwrap();
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
        let (mailer, _) = setup(test, &[("AUTOXIDE_MAIL_FILE_DIR", path.as_str())]).await;

        let error = mailer.send(welcome("not an address")).await.unwrap_err();

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
        let (mailer, worker) = setup(test, &[("AUTOXIDE_MAIL_SMTP_URL", url.as_str())]).await;

        mailer.send(welcome("ada@example.com")).await.unwrap();
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
        let (mailer, worker) = setup(test, &[("AUTOXIDE_MAIL_FILE_DIR", path.as_str())]).await;
        let mut mail = welcome("ada@example.com");
        mail.reply_to = Some("Bob <bob@example.com>".to_owned());
        mail.headers = vec![
            ("List-Unsubscribe".to_owned(), "<https://api.example.com/u/abc>".to_owned()),
            ("List-Unsubscribe-Post".to_owned(), "List-Unsubscribe=One-Click".to_owned()),
        ];
        mail.dedupe_key = Some("delivery-1".to_owned());
        mailer.send(mail.clone()).await.unwrap();
        mailer.send(mail.clone()).await.unwrap();
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
        let error = mailer.send(bad).await.unwrap_err();
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
        mailer.send(mail).await.unwrap();
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
        mailer.send(welcome("ada@example.com")).await.unwrap();
        assert_eq!(worker.run_once().await.unwrap(), 1);
        let done =
            worker.list("mail", autoxide_kernel::job::JobStatus::Completed, 10).await.unwrap();
        assert!(done.is_empty(), "will be retried");
        assert!(failures(&database).await.is_empty());
    })
    .await
    .unwrap();
}
