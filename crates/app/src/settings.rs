//! Cấu hình của cả chương trình: kernel cộng từng module đã đăng ký.
//!
//! Mọi lệnh của CLI đi qua [`load`], nên cấu hình sai bị chặn như nhau ở mọi lệnh,
//! và lỗi của kernel lẫn của mọi module hiện chung trong một thông báo.

use std::path::Path;

use autoxide_kernel::config::{
    Config, ConfigError, ConfigErrors, ConfigReader, ConfigSource, Invalid, SettingValue,
};
use autoxide_kernel::module::{MODULES, Module, Modules, Selection, SelectionError};

use crate::registry;
use crate::startup::StartupConfig;

/// Tên file cấu hình cục bộ, tìm trong thư mục đang đứng.
pub const DOTENV_FILE: &str = ".env";

/// Chương trình đã đọc xong cấu hình: cấu hình của kernel, các module đã nhận cấu
/// hình của mình, và danh sách mọi biến đã đọc (để in ra).
#[derive(Debug)]
pub struct Settings {
    pub config: Config,
    pub modules: Modules,
    /// Việc `serve` làm lúc khởi động (migrate, quản trị viên đầu tiên, task) — `startup.rs`.
    pub startup: StartupConfig,
    pub values: Vec<SettingValue>,
}

/// Lấy tập module nào khi đọc cấu hình.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Choice {
    /// Theo `AUTOXIDE_MODULES`; không đặt thì module chính thức. Mọi lệnh của bản cài dùng cái này.
    Configured,
    /// Theo `AUTOXIDE_MODULES`; không đặt thì MỌI module. Cho test của crate này: chúng thử cả
    /// module thử nghiệm mà không phải nêu biến ở từng bài.
    #[cfg(test)]
    AllUnlessConfigured,
    /// Mọi module, bỏ qua `AUTOXIDE_MODULES`: hợp đồng đầy đủ (`docs/openapi.json`,
    /// `docs/mcp-tools.json`, `.env.example`) không phụ thuộc bản cài đang bật gì.
    Everything,
}

/// Kết quả một lượt đọc: thứ đã đọc được, hoặc `Invalid` khi lỗi đã ghi vào reader.
type Read = Result<(Config, StartupConfig, Modules), Invalid>;

/// Đọc toàn bộ cấu hình từ `reader`, đăng ký những module của `modules` mà `choice` bật.
///
/// Thứ tự đọc cũng là thứ tự trong `.env.example`: kernel, việc lúc khởi động, `AUTOXIDE_MODULES`,
/// rồi từng module đang bật. `Err(ConfigErrors)`: module khai sai (tên, phụ thuộc) — lỗi của
/// người viết module, không đọc tiếp được.
fn read_selected(
    reader: &mut ConfigReader<'_>,
    source: &ConfigSource,
    modules: Vec<Box<dyn Module>>,
    choice: Choice,
) -> Result<Read, ConfigErrors> {
    let config = Config::read(reader);
    let startup = StartupConfig::read(reader);
    // Luôn đọc qua `reader` để biến hiện ở `autoxide config` và `.env.example`.
    let configured = reader.get_with(&MODULES, Selection::parse);
    let unset = source.get(MODULES.name).is_none();
    let selection = match choice {
        Choice::Everything => Ok(Selection::all()),
        #[cfg(test)]
        Choice::AllUnlessConfigured if unset => Ok(Selection::all()),
        _ if unset => Ok(Selection::official()),
        _ => configured,
    };
    let registered = match selection.map(|selection| Modules::select(modules, &selection)) {
        Ok(Ok(modules)) => Ok(modules),
        // Người cài viết sai biến: báo cùng lượt với mọi lỗi cấu hình khác.
        Ok(Err(SelectionError::Setting(problem))) => reader.reject(&MODULES, problem),
        Ok(Err(SelectionError::Modules(error))) => {
            return Err(ConfigErrors::from(ConfigError::new("modules", error.message())));
        }
        // `SelectionError` là `#[non_exhaustive]`: loại lỗi thêm sau này báo nguyên văn.
        Ok(Err(other)) => {
            return Err(ConfigErrors::from(ConfigError::new("modules", other.to_string())));
        }
        Err(Invalid) => Err(Invalid),
    };
    let Ok(mut modules) = registered else {
        return Ok(Err(Invalid));
    };
    // Đọc hết rồi mới `?`: lỗi của kernel và của mọi module hiện cùng lúc.
    let configured = modules.configure(reader);
    Ok((|| {
        configured?;
        Ok((config?, startup?, modules))
    })())
}

fn settings(
    source: &ConfigSource,
    modules: Vec<Box<dyn Module>>,
    choice: Choice,
) -> Result<Settings, ConfigErrors> {
    let mut reader = ConfigReader::new(source);
    let read = read_selected(&mut reader, source, modules, choice)?;
    let ((config, startup, modules), values) = reader.finish(read)?;
    Ok(Settings { config, modules, startup, values })
}

/// Đọc và kiểm toàn bộ cấu hình từ `source` cho các module `modules` — của repo này hay viết ngoài
/// repo. `AUTOXIDE_MODULES` chọn module nào trong đó chạy (mặc định: module chính thức).
pub fn read_with(
    source: &ConfigSource,
    modules: Vec<Box<dyn Module>>,
) -> Result<Settings, ConfigErrors> {
    settings(source, modules, Choice::Configured)
}

/// Như [`read_with`] với các module của chương trình, cho test: không đặt `AUTOXIDE_MODULES` thì
/// MỌI module chạy (kể cả thử nghiệm), để bài test không phải nêu biến.
#[cfg(test)]
pub fn read(source: &ConfigSource) -> Result<Settings, ConfigErrors> {
    settings(source, registry::all(), Choice::AllUnlessConfigured)
}

/// Đọc cấu hình từ môi trường của tiến trình và file `.env` trong thư mục hiện tại.
pub fn load() -> Result<Settings, ConfigErrors> {
    read_with(&ConfigSource::load(Path::new(DOTENV_FILE))?, registry::all())
}

/// Nguồn cấu hình (biến môi trường và `.env`) để một lệnh nhỏ đọc đúng vài biến của nó qua `ConfigReader`, không dựng cả
/// [`Settings`] (database, secret…): `autoxide healthcheck`, `autoxide initial-password`.
pub fn source() -> Result<ConfigSource, ConfigErrors> {
    ConfigSource::load(Path::new(DOTENV_FILE))
}

/// Đọc đúng một biến: `read` nhận reader và trả giá trị đã đọc.
pub fn read_one<T>(
    source: &ConfigSource,
    read: impl FnOnce(&mut ConfigReader<'_>) -> Result<T, autoxide_kernel::config::Invalid>,
) -> Result<T, ConfigErrors> {
    let mut reader = ConfigReader::new(source);
    let value = read(&mut reader);
    reader.finish(value).map(|(value, _)| value)
}

/// Như [`load`] nhưng với MỌI module, bỏ qua `AUTOXIDE_MODULES`: cho lệnh in hợp đồng đầy đủ
/// (`autoxide openapi`, `autoxide mcp-manifest`).
pub fn load_everything() -> Result<Settings, ConfigErrors> {
    let source = ConfigSource::load(Path::new(DOTENV_FILE))?;
    settings(&source, registry::all(), Choice::Everything)
}

/// Biến `AUTOXIDE_…` có trong môi trường hay `.env` mà chương trình không đọc (thường là gõ sai tên), kèm tên gần
/// giống nhất nếu có. Chương trình bỏ qua biến lạ lặng lẽ (người dùng đặt `AUTOXIDE_PORT` rồi thấy máy chủ vẫn nghe
/// cổng mặc định), nên `config` và `serve` nói ra.
pub fn unknown_variables() -> Vec<(String, Option<String>)> {
    let Ok(source) = ConfigSource::load(Path::new(DOTENV_FILE)) else { return Vec::new() };
    let known: Vec<&'static str> = describe().iter().map(|value| value.setting.name).collect();
    unknown_in(&source, &known)
}

/// Biến của `compose` hay công cụ khác dùng chung tiền tố `AUTOXIDE_`, không phải cấu hình của chương trình.
const NOT_SETTINGS: &[&str] = &["AUTOXIDE_IMAGE"];

fn unknown_in(source: &ConfigSource, known: &[&str]) -> Vec<(String, Option<String>)> {
    source
        .names()
        .filter(|name| {
            name.starts_with("AUTOXIDE_") && !known.contains(name) && !NOT_SETTINGS.contains(name)
        })
        .map(|name| {
            let nearest = known
                .iter()
                .map(|candidate| (edit_distance(name, candidate), *candidate))
                .filter(|(distance, _)| *distance <= 3)
                .min()
                .map(|(_, candidate)| candidate.to_owned());
            (name.to_owned(), nearest)
        })
        .collect()
}

/// Khoảng cách Levenshtein, đủ cho gợi ý "ý bạn là…" (tên biến ngắn, vài trăm ứng viên).
fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let current = row[j + 1];
            row[j + 1] = (previous + usize::from(ca != *cb)).min(row[j] + 1).min(current + 1);
            previous = current;
        }
    }
    row[b.len()]
}

/// Danh sách mọi biến chương trình đọc (kernel và mọi module), với giá trị mặc định
/// — không phụ thuộc cấu hình hiện tại, kể cả khi nó đang sai. Dùng để sinh
/// `.env.example`.
pub fn describe() -> Vec<SettingValue> {
    let source = ConfigSource::default();
    let mut reader = ConfigReader::new(&source);
    // Bỏ qua kết quả: biến bắt buộc đương nhiên "thiếu" ở đây; chỉ cần danh sách.
    let _ = read_selected(&mut reader, &source, registry::all(), Choice::Everything);
    reader.values().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_kernel::config::Setting;

    #[test]
    fn bien_la_duoc_bao_kem_goi_y() {
        let source = ConfigSource::from_pairs([
            ("AUTOXIDE_HTTP_ADRESS", "0.0.0.0:80"),
            ("AUTOXIDE_PORT", "9000"),
            ("AUTOXIDE_HTTP_ADDRESS", "0.0.0.0:8080"),
            ("AUTOXIDE_IMAGE", "x"),
            ("PATH", "/bin"),
        ]);
        let known = ["AUTOXIDE_HTTP_ADDRESS", "AUTOXIDE_LOG_LEVEL"];
        let mut unknown = unknown_in(&source, &known);
        unknown.sort();
        assert_eq!(
            unknown,
            vec![
                ("AUTOXIDE_HTTP_ADRESS".to_owned(), Some("AUTOXIDE_HTTP_ADDRESS".to_owned())),
                ("AUTOXIDE_PORT".to_owned(), None),
            ]
        );
    }

    #[test]
    fn moi_bien_cua_env_example_la_bien_da_biet() {
        // Không có biến nào trong danh sách đã biết bị coi là lạ (tên trong `.env.example` đều được đọc).
        let known: Vec<&str> = describe().iter().map(|value| value.setting.name).collect();
        let source = ConfigSource::from_pairs(known.iter().map(|name| (*name, "x")));
        assert!(unknown_in(&source, &known).is_empty());
    }

    const GREETING: Setting =
        Setting::new("AUTOXIDE_HELLO_GREETING", "Greeting.").with_default("hi");
    const TOKEN: Setting = Setting::new("AUTOXIDE_HELLO_TOKEN", "Token.").secret();

    #[derive(Default)]
    struct Hello {
        require_token: bool,
    }

    impl Module for Hello {
        fn name(&self) -> &'static str {
            "hello"
        }

        fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
            let greeting = reader.get::<String>(&GREETING);
            let token =
                if self.require_token { reader.get::<String>(&TOKEN).map(Some) } else { Ok(None) };
            let (_, _) = (greeting?, token?);
            Ok(())
        }
    }

    fn hello(require_token: bool) -> Vec<Box<dyn Module>> {
        vec![Box::new(Hello { require_token })]
    }

    #[test]
    fn bien_cua_module_duoc_doc_cung_bien_cua_kernel() {
        let source = ConfigSource::from_pairs([("AUTOXIDE_HELLO_GREETING", "xin chào")]);

        let settings = read_with(&source, hello(false)).unwrap();

        assert_eq!(settings.modules.names(), ["kernel", "antispam", "hello"]);
        let names: Vec<&str> = settings.values.iter().map(|value| value.setting.name).collect();
        assert_eq!(names.first(), Some(&"DATABASE_URL"));
        assert_eq!(names.last(), Some(&"AUTOXIDE_HELLO_GREETING"));
    }

    #[test]
    fn loi_cua_kernel_va_module_hien_chung_mot_thong_bao() {
        let source = ConfigSource::from_pairs([("DATABASE_URL", "mysql://x")]);

        let message = read_with(&source, hello(true)).unwrap_err().to_string();

        assert!(message.contains("invalid configuration (2 problems):"), "{message}");
        assert!(message.contains("DATABASE_URL:"), "{message}");
        assert!(message.contains("AUTOXIDE_HELLO_TOKEN: is required but not set"), "{message}");
    }

    #[test]
    fn ten_module_sai_la_loi_cau_hinh() {
        struct Bad;
        impl Module for Bad {
            fn name(&self) -> &'static str {
                "Not Valid"
            }
        }

        let message =
            read_with(&ConfigSource::default(), vec![Box::new(Bad)]).unwrap_err().to_string();

        assert!(message.contains("- modules: module name `Not Valid` must be"), "{message}");
    }
}
