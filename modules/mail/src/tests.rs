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
    let worker = modules.worker(&database, 4).unwrap();
    (mailer, worker)
}

fn welcome(to: &str) -> Mail {
    Mail {
        to: to.to_owned(),
        subject: "Welcome".to_owned(),
        text: "Hello Ada, your account is ready.".to_owned(),
        html: Some("<p>Hello Ada, your account is ready.</p>".to_owned()),
    }
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
