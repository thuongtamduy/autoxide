//! Cấu hình của cả chương trình: kernel cộng từng module đã đăng ký.
//!
//! Mọi lệnh của CLI đi qua [`load`], nên cấu hình sai bị chặn như nhau ở mọi lệnh,
//! và lỗi của kernel lẫn của mọi module hiện chung trong một thông báo.

use std::path::Path;

use autoxide_kernel::config::{
    Config, ConfigError, ConfigErrors, ConfigReader, ConfigSource, Invalid, SettingValue,
};
use autoxide_kernel::module::{Module, Modules};

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

/// Đọc cấu hình của kernel, của việc lúc khởi động, rồi của từng module, bằng cùng một `reader`.
fn read_all(
    reader: &mut ConfigReader,
    modules: &mut Modules,
) -> Result<(Config, StartupConfig), Invalid> {
    let config = Config::read(reader);
    let startup = StartupConfig::read(reader);
    let configured = modules.configure(reader);
    configured?;
    Ok((config?, startup?))
}

/// Đăng ký `modules`; tên module sai được báo như một lỗi cấu hình.
fn register(modules: Vec<Box<dyn Module>>) -> Result<Modules, ConfigErrors> {
    Modules::new(modules)
        .map_err(|error| ConfigErrors::from(ConfigError::new("modules", error.message())))
}

/// Đọc và kiểm toàn bộ cấu hình từ `source` cho các module `modules`.
pub fn read_with(
    source: &ConfigSource,
    modules: Vec<Box<dyn Module>>,
) -> Result<Settings, ConfigErrors> {
    let mut modules = register(modules)?;
    let mut reader = ConfigReader::new(source);
    let read = read_all(&mut reader, &mut modules);
    let ((config, startup), values) = reader.finish(read)?;
    Ok(Settings { config, modules, startup, values })
}

/// Như [`read_with`], với các module đã đăng ký của chương trình.
pub fn read(source: &ConfigSource) -> Result<Settings, ConfigErrors> {
    read_with(source, registry::all())
}

/// Đọc cấu hình từ môi trường của tiến trình và file `.env` trong thư mục hiện tại.
pub fn load() -> Result<Settings, ConfigErrors> {
    read(&ConfigSource::load(Path::new(DOTENV_FILE))?)
}

/// Danh sách mọi biến chương trình đọc (kernel và mọi module), với giá trị mặc định
/// — không phụ thuộc cấu hình hiện tại, kể cả khi nó đang sai. Dùng để sinh
/// `.env.example`.
pub fn describe() -> Vec<SettingValue> {
    let source = ConfigSource::default();
    let mut reader = ConfigReader::new(&source);
    // Bỏ qua kết quả: biến bắt buộc đương nhiên "thiếu" ở đây; chỉ cần danh sách.
    if let Ok(mut modules) = register(registry::all()) {
        let _ = read_all(&mut reader, &mut modules);
    }
    reader.values().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_kernel::config::Setting;

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
