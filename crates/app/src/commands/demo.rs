//! Lệnh `autoxide demo`: chạy thử Autoxide bằng một lệnh, không phải cấu hình gì (bước 71, G6).
//!
//! Cách làm: dựng cấu hình bằng các giá trị mặc định riêng của demo (`ConfigSource::with_default`,
//! nên biến môi trường và `.env` của người dùng vẫn thắng), rồi chạy đúng đường của `autoxide serve`.
//! Không có code khởi động thứ hai: demo chỉ là `serve` với cấu hình điền sẵn.
//!
//! Mọi thứ nằm trong một thư mục (mặc định `autoxide-demo`): database SQLite, file, thư mục thư
//! gửi đi, và file `demo-state` giữ khoá bí mật cùng mật khẩu quản trị đã sinh — để lần chạy sau
//! đăng nhập lại được với cùng dữ liệu. Chương trình đổi thư mục làm việc sang thư mục đó, vì
//! các đường dẫn mặc định của cấu hình (`data/…`) là đường dẫn tương đối.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use autoxide_kernel::config::ConfigSource;
use autoxide_kernel::http;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use super::{report_failure, report_invalid_configuration, run_with_observability};
use crate::settings::{self, DOTENV_FILE};
use crate::startup::PasswordSource;

/// Tên file giữ bí mật của demo, trong thư mục demo.
const STATE_FILE: &str = "demo-state";

/// Database của demo: file SQLite trong thư mục demo (đường dẫn tương đối, sau khi đã đổi thư mục).
const DEMO_DATABASE_URL: &str = "sqlite://data/autoxide.sqlite";

/// Email quản trị của demo.
const ADMIN_EMAIL: &str = "admin@example.com";

/// Dữ liệu tham chiếu nạp lúc khởi động (cùng danh sách với `compose.quickstart.yml`).
const SAMPLE_DATA_TASKS: &str = "address.import,tax.import_pack,promotions.load_legal_data";

#[derive(Debug, clap::Args)]
#[command(after_help = "\
Port busy? The demo listens on 127.0.0.1:8080. Set another address before the command:\n  \
AUTOXIDE_HTTP_ADDRESS=127.0.0.1:8081 autoxide demo\n\
(on Windows PowerShell: $env:AUTOXIDE_HTTP_ADDRESS = \"127.0.0.1:8081\"; .\\autoxide.exe demo).\n\
With Docker, change the published port instead: -p 127.0.0.1:8081:8080.")]
pub struct Arguments {
    /// Directory that holds the demo's database, files and secrets. Created when missing;
    /// running the command again with the same directory continues with the same data.
    #[arg(long, default_value = "autoxide-demo")]
    dir: PathBuf,

    /// Use this database instead of the SQLite file in the demo directory. By default the demo IGNORES
    /// `DATABASE_URL` from the environment and `.env`, so it never touches a real database by accident;
    /// the migrations, reference data and the administrator are written to the database you name here.
    #[arg(long, value_name = "URL")]
    database_url: Option<String>,

    /// Keep the outside services from your environment and `.env` (SMTP mail, S3 file storage, OpenTelemetry,
    /// AI providers). By default the demo turns them off: mail is written to files and uploads stay in the demo
    /// directory, so a try-out never sends a real email or writes to a real bucket.
    #[arg(long)]
    keep_external_services: bool,
}

/// Địa chỉ nghe mặc định của server (cùng giá trị với `AUTOXIDE_HTTP_ADDRESS`).
const DEFAULT_ADDRESS: &str = "127.0.0.1:8080";

/// Kiểm cổng còn trống TRƯỚC khi tạo thư mục, sinh mật khẩu hay in banner: cổng bận thì không in gì
/// gây hiểu nhầm là đã chạy. Giá trị không đọc được thì bỏ qua — bước nạp cấu hình sẽ báo lỗi đúng chỗ.
fn check_address_free(configured: Option<&str>) -> Result<(), String> {
    let text = configured.unwrap_or(DEFAULT_ADDRESS);
    let Ok(address) = text.parse::<std::net::SocketAddr>() else { return Ok(()) };
    match std::net::TcpListener::bind(address) {
        Ok(_) => Ok(()),
        Err(error) => Err(format!(
            "cannot listen on `{address}`: {error}. Another program (maybe a previous demo) is using the port. \
             Pick a free one: AUTOXIDE_HTTP_ADDRESS=127.0.0.1:8081 autoxide demo \
             (with Docker: -p 127.0.0.1:8081:8080, the port inside the container stays 8080)."
        )),
    }
}

/// Giá trị demo sinh một lần và giữ lại giữa các lần chạy.
struct State {
    token_secret: String,
    secret_key: String,
    admin_password: String,
}

pub fn run(arguments: &Arguments) -> ExitCode {
    // `.env` đọc từ thư mục hiện tại TRƯỚC khi đổi thư mục: người dùng muốn ghi đè gì thì đặt ở đó.
    let source = match ConfigSource::load(Path::new(DOTENV_FILE)) {
        Ok(source) => source,
        Err(errors) => return report_invalid_configuration(&errors),
    };
    if let Err(error) = check_address_free(source.get("AUTOXIDE_HTTP_ADDRESS")) {
        return report_failure(&error);
    }
    // Đường dẫn đầy đủ của thư mục demo, tính TRƯỚC khi đổi thư mục làm việc (banner cần in đúng chỗ file nằm).
    let demo_dir = std::path::absolute(&arguments.dir).unwrap_or_else(|_| arguments.dir.clone());
    if let Err(error) = std::fs::create_dir_all(&arguments.dir) {
        return report_failure(&format!("cannot create `{}`: {error}", arguments.dir.display()));
    }
    let state_path = arguments.dir.join(STATE_FILE);
    let had_state = state_path.exists();
    let state = match load_or_create_state(&state_path) {
        Ok(state) => state,
        Err(error) => return report_failure(&error),
    };
    if let Err(error) = std::env::set_current_dir(&arguments.dir) {
        return report_failure(&format!("cannot enter `{}`: {error}", arguments.dir.display()));
    }
    let source = demo_source(
        source,
        &state,
        arguments.database_url.as_deref(),
        arguments.keep_external_services,
    );
    let settings = match settings::read_with(&source, crate::registry::all()) {
        Ok(settings) => settings,
        Err(errors) => return report_invalid_configuration(&errors),
    };
    // Banner dựng từ cấu hình ĐÃ NẠP, không từ giá trị demo định đặt: người dùng ghi đè email hay mật khẩu
    // quản trị thì banner nói đúng thứ đang chạy.
    let database_existed =
        settings.config.database.url.sqlite_path().is_some_and(|path| path.exists());
    let banner = Banner {
        dir: demo_dir.clone(),
        address: settings.config.http.address.to_string(),
        docs: settings.config.http.api_docs,
        database: match (&arguments.database_url, settings.config.database.url.sqlite_path()) {
            (Some(_), _) => "the database given with --database-url".to_owned(),
            (None, Some(path)) => format!("SQLite file {}", demo_dir.join(path).display()),
            (None, None) => "SQLite".to_owned(),
        },
        admin: settings.startup.admin.as_ref().map(|admin| {
            let password = match &admin.password {
                PasswordSource::Value(value) => Some(value.clone()),
                PasswordSource::File(_) | PasswordSource::Generate => None,
            };
            (admin.email.clone(), password)
        }),
        // Database đã có dữ liệu mà không còn file bí mật cũ: quản trị viên cũ giữ mật khẩu cũ.
        existing_admin_unknown: database_existed && !had_state,
        external_services: arguments.keep_external_services,
    };
    print!("{}", banner.render());
    let config = settings.config.clone();
    match run_with_observability(&config, super::serve::serve(settings, http::shutdown_signal())) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => report_failure(&error),
    }
}

/// Cấu hình của demo trên cấu hình của người dùng. Hai nhóm:
/// - **Ép** (ghi đè mọi thứ người dùng đặt): database (không bao giờ chạm database thật khi không có
///   `--database-url`), bản sao đọc, migrate lúc khởi động (database demo mới trống), định dạng log (người đọc terminal).
/// - **Mặc định** (người dùng ghi đè được): khoá, quản trị viên, dữ liệu tham chiếu, trang tài liệu API.
fn demo_source(
    source: ConfigSource,
    state: &State,
    database_url: Option<&str>,
    keep_external_services: bool,
) -> ConfigSource {
    let mut source = match database_url {
        Some(url) => source.with_value("DATABASE_URL", url),
        None => source.with_value("DATABASE_URL", DEMO_DATABASE_URL).without("DATABASE_READ_URL"),
    };
    if !keep_external_services {
        // Dịch vụ ngoài: thư gửi đi, kho file, telemetry, nhà cung cấp AI. Bỏ chúng thì mail ghi ra file `data/mail`
        // và file nằm ở `data/files` (mặc định của các module đó). Module bộ nối nhà cung cấp là module thử nghiệm,
        // tắt trừ khi `AUTOXIDE_MODULES` nêu tên: ép về `official`.
        for name in EXTERNAL_SERVICE_SETTINGS {
            source = source.without(name);
        }
        source = source.with_value("AUTOXIDE_MODULES", "official");
    }
    // Mật khẩu đã đặt bằng file thì không đặt thêm mặc định `_PASSWORD`: cấu hình cấm đặt cả hai.
    let password_file_set = source.get("AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD_FILE").is_some();
    let source = if password_file_set {
        source
    } else {
        source.with_default("AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD", &state.admin_password)
    };
    source
        .with_value("AUTOXIDE_MIGRATE_ON_START", "true")
        .with_value("AUTOXIDE_LOG_FORMAT", "text")
        .with_default("AUTOXIDE_AUTH_TOKEN_SECRET", &state.token_secret)
        .with_default("AUTOXIDE_SECRET_KEY", &state.secret_key)
        .with_default("AUTOXIDE_BOOTSTRAP_ADMIN_EMAIL", ADMIN_EMAIL)
        .with_default("AUTOXIDE_STARTUP_TASKS", SAMPLE_DATA_TASKS)
        .with_default("AUTOXIDE_HTTP_API_DOCS", "true")
}

/// Biến cấu hình trỏ ra dịch vụ bên ngoài: thư (SMTP), kho file (S3 và thông tin đăng nhập AWS), OpenTelemetry, nhà
/// cung cấp AI và embedding.
const EXTERNAL_SERVICE_SETTINGS: &[&str] = &[
    "AUTOXIDE_MAIL_SMTP_URL",
    "AUTOXIDE_FILES_S3_BUCKET",
    "AUTOXIDE_FILES_S3_PREFIX",
    "AWS_ENDPOINT_URL",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_REGION",
    "OTEL_EXPORTER_OTLP_ENDPOINT",
    "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
    "AUTOXIDE_AI_API_KEY",
    "AUTOXIDE_AI_BASE_URL",
    "AUTOXIDE_EMBEDDING_API_KEY",
    "AUTOXIDE_EMBEDDING_BASE_URL",
];

struct Banner {
    dir: PathBuf,
    address: String,
    docs: bool,
    database: String,
    /// Email và (nếu biết) mật khẩu của quản trị viên theo cấu hình đã nạp.
    admin: Option<(String, Option<String>)>,
    existing_admin_unknown: bool,
    /// `--keep-external-services`: dịch vụ ngoài từ cấu hình đang bật.
    external_services: bool,
}

impl Banner {
    fn render(&self) -> String {
        // `0.0.0.0` (trong Docker) là địa chỉ nghe, không phải địa chỉ để mở bằng trình duyệt; in `127.0.0.1` như
        // bản chạy trực tiếp để hai cách chạy cho cùng một đường dẫn (README cũng publish cổng ở 127.0.0.1).
        let shown = self.address.replace("0.0.0.0", "127.0.0.1").replace("[::]", "127.0.0.1");
        let base = format!("http://{shown}");
        let mut text = String::new();
        text.push_str("\nAutoxide demo (for trying out, not for production)\n\n");
        text.push_str(&format!("  Data directory  {}\n", self.dir.display()));
        text.push_str(&format!("  Database        {}\n", self.database));
        text.push_str(&format!("  Server          {base}/\n"));
        if self.docs {
            text.push_str(&format!("  API reference   {base}/api/docs\n"));
        }
        text.push_str(&format!("  OpenAPI         {base}/api/v1/openapi.json\n\n"));
        match (&self.admin, self.existing_admin_unknown) {
            (Some((email, _)), true) => {
                text.push_str(&format!("  Administrator   {email}\n"));
                text.push_str(
                    "  Password        unchanged: this database already had data, so the\n",
                );
                text.push_str(
                    "                  administrator keeps the password it was created with.\n",
                );
                text.push_str("                  Delete the data directory to start over.\n\n");
            }
            (Some((email, Some(password))), false) => {
                text.push_str(&format!("  Administrator   {email}\n"));
                text.push_str(&format!("  Password        {password}\n\n"));
            }
            (Some((email, None)), false) => {
                text.push_str(&format!("  Administrator   {email}\n"));
                text.push_str("  Password        as set in your configuration (a file or generated, see the log)\n\n");
            }
            (None, _) => text.push_str(
                "  No administrator is created (AUTOXIDE_BOOTSTRAP_ADMIN_EMAIL is empty).\n\n",
            ),
        }
        if self.external_services {
            text.push_str("WARNING: --keep-external-services is on: mail, file storage, telemetry and AI providers\n");
            text.push_str("from your configuration are ACTIVE. Invitations and uploads reach the real services.\n\n");
        } else {
            text.push_str("Mail is written to files (data/mail) and uploads stay in data/files: no real email is sent,\n");
            text.push_str("no bucket is touched, nothing is exported (pass --keep-external-services to change that).\n\n");
        }
        text.push_str(
            "The reference data (addresses, taxes, promotion limits) is loaded on start.\n",
        );
        text.push_str(
            "Stop with Ctrl+C. Run the command again to continue with the same data.\n\n",
        );
        text
    }
}

/// Đọc file trạng thái; chưa có thì sinh bí mật ngẫu nhiên và ghi (quyền chỉ chủ đọc được).
fn load_or_create_state(path: &Path) -> Result<State, String> {
    if let Ok(text) = std::fs::read_to_string(path) {
        return parse_state(&text).ok_or_else(|| {
            format!("`{}` is damaged: delete the demo directory and start again", path.display())
        });
    }
    let state = State {
        token_secret: random_token(48)?,
        secret_key: random_token(48)?,
        admin_password: random_token(18)?,
    };
    let text = format!("{}\n{}\n{}\n", state.token_secret, state.secret_key, state.admin_password);
    crate::startup::write_private(path, &text)
        .map_err(|error| format!("cannot write `{}`: {error}", path.display()))?;
    Ok(state)
}

fn parse_state(text: &str) -> Option<State> {
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let state = State {
        token_secret: lines.next()?.to_owned(),
        secret_key: lines.next()?.to_owned(),
        admin_password: lines.next()?.to_owned(),
    };
    Some(state)
}

/// `bytes` byte ngẫu nhiên của hệ điều hành, mã hoá base64 an toàn cho URL.
fn random_token(bytes: usize) -> Result<String, String> {
    let mut buffer = vec![0u8; bytes];
    getrandom::fill(&mut buffer).map_err(|error| format!("cannot generate a secret: {error}"))?;
    Ok(URL_SAFE_NO_PAD.encode(buffer))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> State {
        State {
            token_secret: "t".repeat(64),
            secret_key: "k".repeat(64),
            admin_password: "pw".into(),
        }
    }

    #[test]
    fn bi_mat_sinh_mot_lan_va_giu_lai() {
        let dir = std::env::temp_dir().join(format!("autoxide-demo-test-{}", std::process::id()));
        let path = dir.join(STATE_FILE);
        let first = load_or_create_state(&path).unwrap();
        let second = load_or_create_state(&path).unwrap();
        assert_eq!(first.admin_password, second.admin_password);
        assert_eq!(first.token_secret, second.token_secret);
        // Đủ dài cho yêu cầu 32 ký tự của khoá ký.
        assert!(first.token_secret.len() >= 32 && first.secret_key.len() >= 32);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_hong_duoc_bao_ro() {
        let dir = std::env::temp_dir().join(format!("autoxide-demo-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(STATE_FILE);
        std::fs::write(&path, "only-one-line\n").unwrap();
        let error = load_or_create_state(&path).err().unwrap();
        assert!(error.contains("damaged"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nguoi_dung_ghi_de_duoc_mac_dinh_nhung_khong_ghi_de_duoc_database() {
        let user = ConfigSource::from_pairs([
            ("AUTOXIDE_HTTP_API_DOCS", "false"),
            ("DATABASE_URL", "postgres://prod/db"),
            ("DATABASE_READ_URL", "postgres://replica/db"),
            ("AUTOXIDE_MIGRATE_ON_START", "false"),
        ]);
        let source = demo_source(user, &state(), None, false);
        assert_eq!(source.get("AUTOXIDE_HTTP_API_DOCS"), Some("false"));
        assert_eq!(source.get("DATABASE_URL"), Some(DEMO_DATABASE_URL));
        assert_eq!(source.get("DATABASE_READ_URL"), None);
        assert_eq!(source.get("AUTOXIDE_MIGRATE_ON_START"), Some("true"));
    }

    #[test]
    fn co_database_url_ro_rang_thi_dung_no() {
        let source =
            demo_source(ConfigSource::default(), &state(), Some("sqlite://other.db"), false);
        assert_eq!(source.get("DATABASE_URL"), Some("sqlite://other.db"));
    }

    #[test]
    fn cau_hinh_cua_demo_hop_le() {
        let source = demo_source(ConfigSource::default(), &state(), None, false);
        settings::read(&source).unwrap();
    }

    fn banner(admin: Option<(String, Option<String>)>, unknown: bool) -> String {
        Banner {
            dir: PathBuf::from("autoxide-demo"),
            address: "0.0.0.0:8080".into(),
            docs: true,
            database: "SQLite file data/autoxide.sqlite".into(),
            admin,
            existing_admin_unknown: unknown,
            external_services: false,
        }
        .render()
    }

    #[test]
    fn cong_ban_thi_bao_loi_va_cach_doi_cong() {
        let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = held.local_addr().unwrap().to_string();
        let error = check_address_free(Some(&address)).unwrap_err();
        assert!(error.contains(&address), "{error}");
        assert!(error.contains("AUTOXIDE_HTTP_ADDRESS=127.0.0.1:"), "{error}");
        drop(held);
        assert!(check_address_free(Some(&address)).is_ok());
        // Giá trị không đọc được để bước nạp cấu hình báo lỗi.
        assert!(check_address_free(Some("không-phải-địa-chỉ")).is_ok());
    }

    #[test]
    fn banner_noi_ro_dich_vu_ngoai() {
        let text = banner(None, false);
        assert!(text.contains("no real email is sent"), "{text}");
    }

    #[test]
    fn banner_in_dia_chi_mo_duoc_va_mat_khau_dang_chay() {
        let text = banner(Some(("a@b.c".into(), Some("pw".into()))), false);
        assert!(text.contains("http://127.0.0.1:8080/api/docs"), "{text}");
        assert!(text.contains("Password        pw"), "{text}");
        assert!(text.contains("SQLite file data/autoxide.sqlite"), "{text}");
    }

    #[test]
    fn banner_khong_bia_mat_khau_khi_admin_cu_con_do() {
        let text = banner(Some(("a@b.c".into(), Some("new".into()))), true);
        assert!(!text.contains("new"), "{text}");
        assert!(text.contains("already had data"), "{text}");
    }

    /// Demo khởi động thật trên SQLite trong một thư mục tạm: migrate, tạo quản trị viên, nạp dữ liệu, rồi dừng.
    ///
    /// Bài này tự mở SQLite thay vì dùng `TestDatabase::run`: demo CHỈ chạy trên SQLite (một file trong thư mục demo,
    /// hay URL do `--database-url` chỉ ra), nên không có nhánh Postgres nào để chạy cả hai; và bài kiểm đúng đường
    /// `demo_source` → `serve` mà người dùng đi, không phải một database do testkit dựng.
    #[test]
    fn demo_khoi_dong_duoc_tren_sqlite() {
        let dir = std::env::temp_dir().join(format!("autoxide-demo-boot-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let state = State {
            token_secret: "t".repeat(64),
            secret_key: "k".repeat(64),
            admin_password: "pw-pw-pw-pw-pw-1".into(),
        };
        let url = format!("sqlite://{}/autoxide.sqlite", dir.display());
        let source = demo_source(
            ConfigSource::from_pairs([("AUTOXIDE_HTTP_ADDRESS", "127.0.0.1:0")]),
            &state,
            Some(&url),
            false,
        );
        let settings = settings::read_with(&source, crate::registry::all()).unwrap();
        let result = crate::commands::block_on(super::super::serve::serve(settings, async {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }));
        assert!(result.is_ok(), "{result:?}");
        assert!(dir.join("autoxide.sqlite").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dich_vu_ngoai_bi_bo_tru_khi_xin_giu() {
        let user = ConfigSource::from_pairs([
            ("AUTOXIDE_MAIL_SMTP_URL", "smtp://real"),
            ("AUTOXIDE_FILES_S3_BUCKET", "real-bucket"),
            ("AWS_ACCESS_KEY_ID", "key"),
            ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://collector"),
            ("AUTOXIDE_AI_API_KEY", "ai"),
            ("AUTOXIDE_MODULES", "official,momo"),
        ]);
        let off = demo_source(user.clone(), &state(), None, false);
        for name in EXTERNAL_SERVICE_SETTINGS {
            assert_eq!(off.get(name), None, "{name}");
        }
        assert_eq!(off.get("AUTOXIDE_MODULES"), Some("official"));
        let kept = demo_source(user, &state(), None, true);
        assert_eq!(kept.get("AUTOXIDE_MAIL_SMTP_URL"), Some("smtp://real"));
        assert_eq!(kept.get("AUTOXIDE_MODULES"), Some("official,momo"));
    }

    #[test]
    fn mat_khau_bang_file_khong_bi_them_mat_khau_mac_dinh() {
        let user = ConfigSource::from_pairs([(
            "AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD_FILE",
            "/run/secrets/admin",
        )]);
        let source = demo_source(user, &state(), None, false);
        assert_eq!(source.get("AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD"), None);
        // Cấu hình vẫn hợp lệ (không báo "not both").
        settings::read(&source).unwrap();
    }
}
