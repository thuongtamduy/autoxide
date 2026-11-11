//! Chạy binary `autoxide` thật như người dùng chạy: kiểm mã thoát và nội dung in ra.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
// Clippy chỉ tự miễn cho hàm `#[test]`, không miễn cho hàm phụ, nên khai ở đây.
// Trong `src/` thì KHÔNG được khai dòng này (`cargo xtask check` chặn).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Thư mục tạm riêng cho một bài test, tự xoá khi xong. Binary được chạy với thư
/// mục này làm thư mục hiện tại, nên file `.env` của repo không ảnh hưởng.
struct WorkDir(PathBuf);

impl WorkDir {
    fn new(label: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("autoxide-cli-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        WorkDir(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Chạy `autoxide <arguments>` trong `directory` với đúng các biến `environment`
/// (không thừa hưởng biến của tiến trình test, để kết quả không phụ thuộc máy).
fn autoxide(directory: &Path, arguments: &[&str], environment: &[(&str, &str)]) -> Output {
    // Cargo đặt biến này lúc biên dịch test: đường dẫn tới binary vừa build.
    Command::new(env!("CARGO_BIN_EXE_autoxide"))
        .args(arguments)
        .current_dir(directory)
        .env_clear()
        .envs(environment.iter().copied())
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

#[test]
fn config_voi_mac_dinh_thi_hop_le() {
    let directory = WorkDir::new("default");

    let output = autoxide(directory.path(), &["config"], &[]);

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let printed = stdout(&output);
    assert!(printed.starts_with("Configuration is valid.\n"), "{printed}");
    assert!(printed.contains("DATABASE_URL = sqlite://data/autoxide.sqlite  [default]"));
}

#[test]
fn config_sai_thi_thoat_ma_2_va_noi_ro_phai_sua_gi() {
    let directory = WorkDir::new("invalid");

    let output = autoxide(
        directory.path(),
        &["config"],
        &[("DATABASE_URL", "mysql://root:s3cr3t@localhost/app")],
    );

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(stdout(&output), "");
    let message = stderr(&output);
    assert!(message.contains("invalid configuration (1 problem):"), "{message}");
    assert!(message.contains("DATABASE_URL: must start with `sqlite:` or `postgres://`"));
    assert!(message.contains(".env.example"), "{message}");
    assert!(!message.contains("s3cr3t"), "{message}");
}

#[test]
fn doc_file_dotenv_trong_thu_muc_hien_tai() {
    let directory = WorkDir::new("dotenv");
    std::fs::write(directory.path().join(".env"), "DATABASE_URL=postgres://app:s3cr3t@db/app\n")
        .unwrap();

    let output = autoxide(directory.path(), &["config"], &[]);

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let printed = stdout(&output);
    assert!(printed.contains("DATABASE_URL = (set, hidden)  [set]"), "{printed}");
    assert!(!printed.contains("s3cr3t"), "{printed}");
}

#[test]
fn bien_moi_truong_thang_file_dotenv() {
    let directory = WorkDir::new("precedence");
    // File `.env` sai; biến môi trường đúng. Biến môi trường phải thắng.
    std::fs::write(directory.path().join(".env"), "DATABASE_URL=mysql://nope\n").unwrap();

    let output =
        autoxide(directory.path(), &["config"], &[("DATABASE_URL", "sqlite://other.sqlite")]);

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
}

#[test]
fn env_example_in_duoc_ca_khi_cau_hinh_dang_sai() {
    let directory = WorkDir::new("example");

    let output =
        autoxide(directory.path(), &["config", "--env-example"], &[("DATABASE_URL", "mysql://x")]);

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let committed =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.env.example"))
            .unwrap();
    assert_eq!(stdout(&output), committed);
}

#[test]
fn khong_co_lenh_thi_in_huong_dan_va_thoat_ma_2() {
    let directory = WorkDir::new("help");

    let output = autoxide(directory.path(), &[], &[]);

    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("Usage: autoxide <COMMAND>"), "{}", stderr(&output));
}

#[test]
fn migrate_voi_cau_hinh_mac_dinh_tu_tao_file_sqlite() {
    let directory = WorkDir::new("migrate-default");

    let status = autoxide(directory.path(), &["migrate", "status"], &[]);
    let up = autoxide(directory.path(), &["migrate", "up"], &[]);

    assert_eq!(status.status.code(), Some(0), "{}", stderr(&status));
    // Không ghim danh sách migration ở đây (module thêm migration là đổi): chỉ kiểm
    // database mới thì mọi migration đều chờ, và `up` áp dụng hết.
    let listed = stdout(&status);
    assert!(listed.starts_with("auth  m0001_create_user "), "{listed}");
    assert!(listed.contains("\n0 applied, "), "{listed}");
    assert_eq!(up.status.code(), Some(0), "{}", stderr(&up));
    assert!(stdout(&up).contains("  auth/m0001_create_user\n"), "{}", stdout(&up));
    let again = autoxide(directory.path(), &["migrate", "up"], &[]);
    assert_eq!(stdout(&again), "Nothing to apply: the database is up to date.\n");
    // Không cấu hình gì: database là file SQLite trong thư mục `data/`, tự tạo.
    assert!(directory.path().join("data/autoxide.sqlite").exists());
}

#[test]
fn migrate_down_module_khong_ton_tai_thoat_ma_1() {
    let directory = WorkDir::new("migrate-unknown");

    let output = autoxide(directory.path(), &["migrate", "down", "nope"], &[]);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stderr(&output),
        "autoxide: migration: unknown module `nope`; registered modules: auth\n"
    );
}

#[test]
fn khong_ket_noi_duoc_database_thoat_ma_1_va_khong_lo_mat_khau() {
    let directory = WorkDir::new("migrate-unreachable");

    // Port 1 không có gì nghe: kết nối bị từ chối ngay.
    let output = autoxide(
        directory.path(),
        &["migrate", "status"],
        &[
            ("DATABASE_URL", "postgres://app:s3cr3t@127.0.0.1:1/app"),
            ("AUTOXIDE_DATABASE_WAIT_SECONDS", "1"),
        ],
    );

    assert_eq!(output.status.code(), Some(1));
    let message = stderr(&output);
    assert!(
        message.starts_with(
            "autoxide: database: cannot connect to `postgres://app:***@127.0.0.1:1/app`"
        ),
        "{message}"
    );
    assert!(!message.contains("s3cr3t"), "{message}");
}

/// Chạy trên database gate đang chọn (SQLite hoặc Postgres).
#[tokio::test]
async fn migrate_chay_tren_database_dang_chon() {
    autoxide_testkit::TestDatabase::run(async |database| {
        let directory = WorkDir::new("migrate-selected");
        let environment = [("DATABASE_URL", database.url())];

        let up = autoxide(directory.path(), &["migrate", "up"], &environment);
        let status = autoxide(directory.path(), &["migrate", "status"], &environment);

        assert_eq!(up.status.code(), Some(0), "{}", stderr(&up));
        assert_eq!(status.status.code(), Some(0), "{}", stderr(&status));
    })
    .await
    .unwrap();
}

#[test]
fn openapi_in_ra_dung_tai_lieu_trong_repo() {
    let directory = WorkDir::new("openapi");

    let output = autoxide(directory.path(), &["openapi"], &[]);

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let committed = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/openapi.json"),
    )
    .unwrap();
    assert_eq!(stdout(&output), committed);
}

/// Chạy `autoxide serve` thật, gọi một endpoint, rồi gửi SIGTERM như Docker hay
/// systemd khi dừng dịch vụ: tiến trình phải dừng êm và thoát mã 0.
#[cfg(unix)]
#[test]
fn serve_phuc_vu_roi_dung_em_khi_nhan_sigterm() {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::process::Stdio;

    let directory = WorkDir::new("serve");
    // `serve` từ chối chạy khi còn migration chưa áp dụng.
    let refused = autoxide(directory.path(), &["serve"], &[]);
    assert_eq!(refused.status.code(), Some(1));
    assert!(stderr(&refused).contains("run `autoxide migrate up` first"), "{}", stderr(&refused));
    let up = autoxide(directory.path(), &["migrate", "up"], &[]);
    assert_eq!(up.status.code(), Some(0), "{}", stderr(&up));

    let mut child = Command::new(env!("CARGO_BIN_EXE_autoxide"))
        .arg("serve")
        .current_dir(directory.path())
        .env_clear()
        // Port 0: hệ điều hành chọn port trống; port thật đọc từ dòng log "listening".
        .envs([("AUTOXIDE_HTTP_ADDRESS", "127.0.0.1:0"), ("AUTOXIDE_LOG_FORMAT", "json")])
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut log = BufReader::new(child.stderr.take().unwrap());

    let mut address = None;
    let mut line = String::new();
    while address.is_none() {
        line.clear();
        if log.read_line(&mut line).unwrap() == 0 {
            let _ = child.kill();
            panic!("`autoxide serve` exited before listening: {:?}", child.wait());
        }
        let event: serde_json::Value = serde_json::from_str(&line).unwrap();
        if event["message"] == "listening" {
            address = event["address"].as_str().map(str::to_owned);
        }
    }
    let address = address.unwrap();

    let mut stream = std::net::TcpStream::connect(&address).unwrap();
    stream
        .write_all(b"GET /health/ready HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();

    let sent = Command::new("kill").args(["-TERM", &child.id().to_string()]).status().unwrap();
    let status = child.wait().unwrap();
    let mut rest = String::new();
    log.read_to_string(&mut rest).unwrap();

    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
    assert!(sent.success());
    assert_eq!(status.code(), Some(0), "{rest}");
    assert!(rest.contains(r#""message":"stopped""#), "{rest}");
}
