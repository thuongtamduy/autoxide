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

/// Tên file giữ bí mật của demo, trong thư mục demo.
const STATE_FILE: &str = "demo-state";

/// Email quản trị của demo.
const ADMIN_EMAIL: &str = "admin@example.com";

/// Dữ liệu tham chiếu nạp lúc khởi động (cùng danh sách với `compose.quickstart.yml`).
const SAMPLE_DATA_TASKS: &str = "address.import,tax.import_pack,promotions.load_legal_data";

#[derive(Debug, clap::Args)]
pub struct Arguments {
    /// Directory that holds the demo's database, files and secrets. Created when missing;
    /// running the command again with the same directory continues with the same data.
    #[arg(long, default_value = "autoxide-demo")]
    dir: PathBuf,
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
    if let Err(error) = std::fs::create_dir_all(&arguments.dir) {
        return report_failure(&format!("cannot create `{}`: {error}", arguments.dir.display()));
    }
    let state = match load_or_create_state(&arguments.dir.join(STATE_FILE)) {
        Ok(state) => state,
        Err(error) => return report_failure(&error),
    };
    if let Err(error) = std::env::set_current_dir(&arguments.dir) {
        return report_failure(&format!("cannot enter `{}`: {error}", arguments.dir.display()));
    }
    let source = demo_source(source, &state);
    let settings = match settings::read_with(&source, crate::registry::all()) {
        Ok(settings) => settings,
        Err(errors) => return report_invalid_configuration(&errors),
    };
    let address = settings.config.http.address;
    let docs = settings.config.http.api_docs;
    print!("{}", banner(&arguments.dir, &address.to_string(), docs, &state));
    let config = settings.config.clone();
    match run_with_observability(&config, super::serve::serve(settings, http::shutdown_signal())) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => report_failure(&error),
    }
}

/// Các giá trị mặc định của demo đặt lên cấu hình của người dùng.
fn demo_source(source: ConfigSource, state: &State) -> ConfigSource {
    source
        .with_default("AUTOXIDE_AUTH_TOKEN_SECRET", &state.token_secret)
        .with_default("AUTOXIDE_SECRET_KEY", &state.secret_key)
        .with_default("AUTOXIDE_MIGRATE_ON_START", "true")
        .with_default("AUTOXIDE_BOOTSTRAP_ADMIN_EMAIL", ADMIN_EMAIL)
        .with_default("AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD", &state.admin_password)
        .with_default("AUTOXIDE_STARTUP_TASKS", SAMPLE_DATA_TASKS)
        .with_default("AUTOXIDE_HTTP_API_DOCS", "true")
        .with_default("AUTOXIDE_LOG_FORMAT", "text")
}

fn banner(dir: &Path, address: &str, docs: bool, state: &State) -> String {
    let base = format!("http://{address}");
    let mut text = String::new();
    text.push_str("\nAutoxide demo (for trying out, not for production)\n\n");
    text.push_str(&format!("  Data directory  {}\n", dir.display()));
    text.push_str(&format!("  Server          {base}/\n"));
    if docs {
        text.push_str(&format!("  API reference   {base}/api/docs\n"));
    }
    text.push_str(&format!("  OpenAPI         {base}/api/v1/openapi.json\n\n"));
    text.push_str(&format!("  Administrator   {ADMIN_EMAIL}\n"));
    text.push_str(&format!("  Password        {}\n\n", state.admin_password));
    text.push_str("The reference data (addresses, taxes, promotion limits) is loaded on start.\n");
    text.push_str("Stop with Ctrl+C. Run the command again to continue with the same data.\n\n");
    text
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
    fn nguoi_dung_ghi_de_duoc_mac_dinh_cua_demo() {
        let user = ConfigSource::from_pairs([("AUTOXIDE_HTTP_API_DOCS", "false")]);
        let source = demo_source(user, &state());
        assert_eq!(source.get("AUTOXIDE_HTTP_API_DOCS"), Some("false"));
        assert_eq!(source.get("AUTOXIDE_MIGRATE_ON_START"), Some("true"));
    }

    #[test]
    fn cau_hinh_cua_demo_hop_le() {
        let source = demo_source(ConfigSource::default(), &state());
        settings::read(&source).unwrap();
    }

    #[test]
    fn banner_co_mat_khau_va_trang_api() {
        let text = banner(Path::new("autoxide-demo"), "127.0.0.1:8080", true, &state());
        assert!(text.contains("Password        pw"));
        assert!(text.contains("http://127.0.0.1:8080/api/docs"));
    }
}
