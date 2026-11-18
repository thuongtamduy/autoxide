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

/// Như [`autoxide`], gửi `input` vào standard input của lệnh.
fn autoxide_with_input(
    directory: &Path,
    arguments: &[&str],
    environment: &[(&str, &str)],
    input: &str,
) -> Output {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = Command::new(env!("CARGO_BIN_EXE_autoxide"))
        .args(arguments)
        .current_dir(directory)
        .env_clear()
        .envs(environment.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    child.wait_with_output().unwrap()
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
    assert!(
        listed
            .lines()
            .any(|line| line.starts_with("auth ") && line.contains(" m0001_create_user ")),
        "{listed}"
    );
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
        "autoxide: migration: unknown module `nope`; registered modules: kernel, antispam, mail, ai, files, address, organizations, auth, audit, idempotency, webhooks, tax, ledger, invoices, compliance, notifications, plans, media, content, forms, newsletter, comments, menus, reporting, automations\n"
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
    // Người dùng đầu tiên, tạo bằng CLI như hướng dẫn cài đặt.
    let created = autoxide_with_input(
        directory.path(),
        &["user", "create", "--email", "ada@example.com", "--name", "Ada", "--admin"],
        &[],
        "",
    );
    assert_eq!(created.status.code(), Some(0), "{}", stderr(&created));
    let password = stdout(&created)
        .lines()
        .find_map(|line| line.strip_prefix("Password: "))
        .unwrap()
        .to_owned();

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

    let http = |method: &str, path: &str, authorization: &str, body: &str| {
        let mut stream = std::net::TcpStream::connect(&address).unwrap();
        let request = format!(
            "{method} {path} HTTP/1.1\r\nHost: test\r\nConnection: close\r\n{authorization}\
             Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(request.as_bytes()).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    };
    let response = http("GET", "/health/ready", "", "");
    // Đăng nhập bằng tài khoản vừa tạo, rồi xem quyền của mình.
    let credentials = serde_json::json!({"email": "ada@example.com", "password": password});
    let login = http("POST", "/api/v1/auth/login", "", &credentials.to_string());
    let (_, body) = login.split_once("\r\n\r\n").unwrap();
    let login_body: serde_json::Value = serde_json::from_str(body).unwrap();
    let token = login_body["data"]["access_token"].as_str().unwrap_or_default().to_owned();
    let me = http("GET", "/api/v1/auth/me", &format!("Authorization: Bearer {token}\r\n"), "");

    let sent = Command::new("kill").args(["-TERM", &child.id().to_string()]).status().unwrap();
    let status = child.wait().unwrap();
    let mut rest = String::new();
    log.read_to_string(&mut rest).unwrap();

    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
    assert!(login.starts_with("HTTP/1.1 200 OK\r\n"), "{login}");
    assert!(me.starts_with("HTTP/1.1 200 OK\r\n"), "{me}");
    assert!(me.contains(r#""all_permissions":true"#), "{me}");
    assert!(sent.success());
    assert_eq!(status.code(), Some(0), "{rest}");
    assert!(rest.contains(r#""message":"stopped""#), "{rest}");
}

/// Tạo người dùng bằng CLI trên database gate đang chọn (SQLite hoặc Postgres).
#[tokio::test]
async fn user_create_tao_admin_va_bao_loi_ro() {
    autoxide_testkit::TestDatabase::run(async |database| {
        let directory = WorkDir::new("user-create");
        let environment = [("DATABASE_URL", database.url())];
        let create = |email: &str, input: &str| {
            let arguments = [
                "user",
                "create",
                "--email",
                email,
                "--name",
                "Ada",
                "--admin",
                "--password-stdin",
            ];
            autoxide_with_input(directory.path(), &arguments, &environment, input)
        };

        let before = create("ada@example.com", "correct horse battery staple\n");
        assert_eq!(before.status.code(), Some(1));
        assert!(stderr(&before).contains("run `autoxide migrate up` first"), "{}", stderr(&before));

        let up = autoxide(directory.path(), &["migrate", "up"], &environment);
        assert_eq!(up.status.code(), Some(0), "{}", stderr(&up));
        let created = create("Ada@Example.com", "correct horse battery staple\n");
        assert_eq!(created.status.code(), Some(0), "{}", stderr(&created));
        let printed = stdout(&created);
        assert!(printed.starts_with("Created user ada@example.com (id "), "{printed}");
        assert!(printed.contains("with the `admin` role"), "{printed}");
        // Mật khẩu do người dùng đưa vào thì không in lại.
        assert!(!printed.contains("correct horse"), "{printed}");

        let taken = create("ada@example.com", "correct horse battery staple\n");
        assert_eq!(taken.status.code(), Some(1));
        assert!(stderr(&taken).contains("(auth.email_taken)"), "{}", stderr(&taken));

        let short = create("bob@example.com", "short\n");
        assert_eq!(short.status.code(), Some(1));
        assert!(stderr(&short).contains("  - password: must be between 12"), "{}", stderr(&short));

        // Gỡ xác thực hai bước (D-159): tài khoản có thật thì xong (kể cả khi chưa bật), không
        // có thì báo rõ.
        let reset = |email: &str| {
            autoxide(directory.path(), &["user", "reset-mfa", "--email", email], &environment)
        };
        let done = reset("ada@example.com");
        assert_eq!(done.status.code(), Some(0), "{}", stderr(&done));
        assert!(stdout(&done).contains("Two-step sign-in of ada@example.com"), "{}", stdout(&done));
        let missing = reset("ghost@example.com");
        assert_eq!(missing.status.code(), Some(1));
        assert!(stderr(&missing).contains("(auth.account_not_found)"), "{}", stderr(&missing));

        // Gỡ mọi passkey (đặc tả 32 B2): như trên.
        let remove = |email: &str| {
            autoxide(directory.path(), &["user", "remove-passkeys", "--email", email], &environment)
        };
        let done = remove("ada@example.com");
        assert_eq!(done.status.code(), Some(0), "{}", stderr(&done));
        assert!(
            stdout(&done).contains("Removed 0 passkey(s) of ada@example.com"),
            "{}",
            stdout(&done)
        );
        let missing = remove("ghost@example.com");
        assert_eq!(missing.status.code(), Some(1));
        assert!(stderr(&missing).contains("(auth.account_not_found)"), "{}", stderr(&missing));
    })
    .await
    .unwrap();
}

/// Lệnh xem và chạy lại job hỏng, trên database gate đang chọn.
#[tokio::test]
async fn jobs_xem_va_chay_lai_job_hong() {
    autoxide_testkit::TestDatabase::run(async |database| {
        let directory = WorkDir::new("jobs");
        let environment = [("DATABASE_URL", database.url())];
        let up = autoxide(directory.path(), &["migrate", "up"], &environment);
        assert_eq!(up.status.code(), Some(0), "{}", stderr(&up));

        let failed = autoxide(directory.path(), &["jobs", "failed"], &environment);
        assert_eq!(failed.status.code(), Some(0), "{}", stderr(&failed));
        assert_eq!(stdout(&failed), "No failed jobs.\n");

        let id = "01936f0e-6c9f-7cc2-8a52-2b2f4d1c1e5a";
        let retry = autoxide(directory.path(), &["jobs", "retry", "auth", id], &environment);
        assert_eq!(retry.status.code(), Some(1));
        assert!(stderr(&retry).contains("has no failed job with id"), "{}", stderr(&retry));
        let unknown =
            autoxide(directory.path(), &["jobs", "failed", "--module", "nope"], &environment);
        assert_eq!(unknown.status.code(), Some(1));
        assert!(
            stderr(&unknown).contains("Module `nope` has no job queue"),
            "{}",
            stderr(&unknown)
        );
    })
    .await
    .unwrap();
}

/// `autoxide copy`: dữ liệu tạo trên SQLite sang database gate đang chọn (lượt
/// Postgres của gate là SQLite → Postgres); đích đã có dữ liệu thì từ chối.
#[tokio::test]
async fn copy_chep_du_lieu_sang_database_khac() {
    autoxide_testkit::TestDatabase::run(async |target| {
        let directory = WorkDir::new("copy");
        let source = [("DATABASE_URL", "sqlite://source.sqlite?mode=rwc")];
        let migrated = autoxide(directory.path(), &["migrate", "up"], &source);
        assert_eq!(migrated.status.code(), Some(0), "{}", stderr(&migrated));
        let arguments =
            ["user", "create", "--email", "ada@example.com", "--name", "Ada", "--admin"];
        let created = autoxide(directory.path(), &arguments, &source);
        assert_eq!(created.status.code(), Some(0), "{}", stderr(&created));

        // URL đích qua standard input: không hiện trong danh sách tiến trình.
        let copied =
            autoxide_with_input(directory.path(), &["copy", "--to-stdin"], &source, target.url());
        assert_eq!(copied.status.code(), Some(0), "{}", stderr(&copied));
        let report = stdout(&copied);
        assert!(report.lines().any(|line| line.trim() == "1  auth_user"), "{report}");
        assert!(report.contains("Copied "), "{report}");
        // Tài khoản đã sang đích: tạo lại cùng email ở đích thì báo trùng.
        let on_target = [("DATABASE_URL", target.url())];
        let duplicate = autoxide(directory.path(), &arguments, &on_target);
        assert_eq!(duplicate.status.code(), Some(1));
        assert!(stderr(&duplicate).contains("already exists"), "{}", stderr(&duplicate));

        let again = autoxide(directory.path(), &["copy", "--to", target.url()], &source);
        assert_eq!(again.status.code(), Some(1));
        assert!(stderr(&again).contains("already has Autoxide tables"), "{}", stderr(&again));
        let neither = autoxide(directory.path(), &["copy"], &source);
        assert_eq!(neither.status.code(), Some(2), "clap rejects a missing target");
    })
    .await
    .unwrap();
}
