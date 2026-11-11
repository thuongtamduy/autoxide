//! Hợp đồng module: những gì một module khai với core, và [`Modules`] — tập
//! module đã đăng ký của chương trình.
//!
//! Một module là một crate trong `modules/<tên>`. Nó mở ra ngoài đúng hai thứ
//! (`cargo xtask check` kiểm):
//!
//! - `pub fn module() -> impl Module` — để binary đăng ký;
//! - `pub mod api` — trait và kiểu dữ liệu cho module khác dùng.
//!
//! Binary đăng ký mọi module ở MỘT chỗ (`crates/app/src/modules.rs`). Core không
//! biết tên module nào: nó chỉ làm việc qua trait [`Module`], nên thêm một module
//! không phải sửa core.
//!
//! Hợp đồng lớn dần theo lộ trình: mỗi phần của core (route, permission, event,
//! job, tool MCP) thêm một hàm vào [`Module`], luôn kèm cài đặt mặc định "không
//! có gì" — module cũ không phải sửa khi hợp đồng thêm hàm mới.

use std::collections::BTreeSet;
use std::fmt;

use sea_orm_migration::MigrationTrait;

use crate::config::{ConfigReader, Invalid};
use crate::migration::{MigrationError, Migrator, ModuleMigrations};

/// Những gì một module khai với core.
///
/// ```
/// use autoxide_kernel::config::{ConfigReader, Invalid, Setting};
/// use autoxide_kernel::module::Module;
///
/// const PAGE_SIZE: Setting =
///     Setting::new("AUTOXIDE_NOTES_PAGE_SIZE", "Notes returned per page.").with_default("50");
///
/// #[derive(Default)]
/// struct NotesModule {
///     page_size: u32,
/// }
///
/// impl Module for NotesModule {
///     fn name(&self) -> &'static str {
///         "notes"
///     }
///
///     fn configure(&mut self, reader: &mut ConfigReader) -> Result<(), Invalid> {
///         self.page_size = reader.get(&PAGE_SIZE)?;
///         Ok(())
///     }
/// }
///
/// // Hàm duy nhất crate của module mở cho binary.
/// pub fn module() -> impl Module {
///     NotesModule::default()
/// }
/// ```
//
// `Send + Sync`: module được dùng từ nhiều thread của máy chủ. Trình biên dịch
// từ chối module chứa thứ không an toàn khi chia sẻ giữa các thread.
pub trait Module: Send + Sync {
    /// Tên module: chữ thường, số và `_`, bắt đầu bằng chữ. Cũng là tiền tố của
    /// mọi bảng (`<tên>_…`) và mọi biến cấu hình (`AUTOXIDE_<TÊN>_…`) của module.
    fn name(&self) -> &'static str;

    /// Các migration của module, theo thứ tự áp dụng. Đã commit thì không sửa —
    /// thêm migration mới vào cuối.
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        Vec::new()
    }

    /// Đọc cấu hình riêng của module và giữ lại trong chính nó. Gọi một lần lúc
    /// khởi động, trước mọi việc khác. Lỗi ghi vào `reader`, được báo chung một
    /// lượt với lỗi cấu hình của kernel và các module khác.
    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        // `let _ =`: tham số không dùng tới trong cài đặt mặc định.
        let _ = reader;
        Ok(())
    }
}

/// Lỗi trong việc đăng ký module.
#[derive(Debug)]
pub struct ModuleError(String);

impl ModuleError {
    /// Nội dung lỗi, không kèm tiền tố `modules:`.
    pub fn message(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ModuleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "modules: {}", self.0)
    }
}

impl std::error::Error for ModuleError {}

/// Tập module đã đăng ký, theo thứ tự đăng ký.
pub struct Modules {
    // `Box<dyn Module>`: mỗi module là một kiểu khác nhau; `dyn` cho phép để chung
    // một danh sách và gọi qua trait.
    modules: Vec<Box<dyn Module>>,
}

impl Modules {
    /// Kiểm tên rồi nhận các module. Tên sai là lỗi của người viết module, báo
    /// ngay lúc khởi động.
    pub fn new(modules: Vec<Box<dyn Module>>) -> Result<Self, ModuleError> {
        let names: Vec<&str> = modules.iter().map(|module| module.name()).collect();
        let problems = module_name_problems(&names);
        if problems.is_empty() {
            Ok(Modules { modules })
        } else {
            Err(ModuleError(problems.join("; ")))
        }
    }

    /// Tên các module, theo thứ tự đăng ký.
    pub fn names(&self) -> Vec<&'static str> {
        self.modules.iter().map(|module| module.name()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }

    /// Cho từng module đọc cấu hình của nó. Module nào sai cũng không chặn module
    /// khác đọc — để mọi lỗi được báo cùng lúc.
    pub fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        let mut result = Ok(());
        for module in &mut self.modules {
            if module.configure(reader).is_err() {
                result = Err(Invalid);
            }
        }
        result
    }

    /// Migrator chạy migration của mọi module.
    pub fn migrator(&self) -> Result<Migrator, MigrationError> {
        Migrator::new(
            self.modules
                .iter()
                .map(|module| ModuleMigrations::new(module.name(), module.migrations()))
                .collect(),
        )
    }
}

impl fmt::Debug for Modules {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Modules").field(&self.names()).finish()
    }
}

/// Tên mà database dành riêng làm tiền tố bảng hệ thống: SQLite giữ `sqlite_…`,
/// Postgres giữ `pg_…`. Module mang tên này sẽ tạo bảng không hợp lệ ở một bên.
const RESERVED_NAMES: [&str; 2] = ["sqlite", "pg"];

/// Các vấn đề của một danh sách tên module; rỗng nghĩa là hợp lệ.
pub(crate) fn module_name_problems(names: &[&str]) -> Vec<String> {
    let mut problems = Vec::new();
    let mut seen = BTreeSet::new();
    for name in names {
        if !is_identifier(name) {
            problems.push(format!(
                "module name `{name}` must be lowercase letters, digits and `_`, starting with a letter"
            ));
        }
        if RESERVED_NAMES.contains(name) {
            problems.push(format!(
                "module name `{name}` is reserved: its tables would use a prefix the database \
                 keeps for system tables"
            ));
        }
        if !seen.insert(*name) {
            problems.push(format!("module `{name}` is registered twice"));
        }
    }
    // `auth` và `auth_keys` cùng tồn tại thì bảng `auth_keys_x` thuộc về ai? Cấm
    // từ đầu để tiền tố bảng luôn chỉ ra đúng một module.
    for first in &seen {
        for second in &seen {
            if second.starts_with(&format!("{first}_")) {
                problems.push(format!(
                    "module names `{first}` and `{second}` are ambiguous: tables of `{second}` \
                     would also carry the prefix of `{first}`"
                ));
            }
        }
    }
    problems
}

/// Chữ thường, số và `_`; bắt đầu bằng chữ; không kết thúc bằng `_`.
pub(crate) fn is_identifier(text: &str) -> bool {
    text.chars().next().is_some_and(|first| first.is_ascii_lowercase())
        && text.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !text.ends_with('_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ConfigSource, Setting};

    const LIMIT: Setting = Setting::new("AUTOXIDE_NOTES_LIMIT", "Limit.").with_default("10");
    const KEY: Setting = Setting::new("AUTOXIDE_TAGS_KEY", "Key.");

    /// Module không khai gì ngoài tên: dùng mọi cài đặt mặc định của hợp đồng.
    struct Bare(&'static str);

    impl Module for Bare {
        fn name(&self) -> &'static str {
            self.0
        }
    }

    /// Module đọc một biến có mặc định.
    #[derive(Default)]
    struct Notes {
        limit: u32,
    }

    impl Module for Notes {
        fn name(&self) -> &'static str {
            "notes"
        }

        fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
            self.limit = reader.get(&LIMIT)?;
            Ok(())
        }
    }

    /// Module đọc một biến bắt buộc.
    struct Tags;

    impl Module for Tags {
        fn name(&self) -> &'static str {
            "tags"
        }

        fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
            reader.get::<String>(&KEY).map(|_| ())
        }
    }

    fn bare(names: &[&'static str]) -> Vec<Box<dyn Module>> {
        names.iter().map(|name| Box::new(Bare(name)) as Box<dyn Module>).collect()
    }

    fn error(names: &[&'static str]) -> String {
        Modules::new(bare(names)).unwrap_err().to_string()
    }

    #[test]
    fn giu_thu_tu_dang_ky() {
        let modules = Modules::new(bare(&["notes", "auth", "tags"])).unwrap();
        assert_eq!(modules.names(), ["notes", "auth", "tags"]);
        assert!(!modules.is_empty());
        assert_eq!(format!("{modules:?}"), r#"Modules(["notes", "auth", "tags"])"#);
        assert!(Modules::new(Vec::new()).unwrap().is_empty());
    }

    #[test]
    fn ten_module_sai_dang_bi_tu_choi() {
        for name in ["", "Auth", "1auth", "auth-keys", "auth_", "au th"] {
            let message = error(&[name]);
            assert!(message.contains("must be lowercase letters, digits and `_`"), "{message}");
        }
    }

    #[test]
    fn ten_danh_rieng_cua_database_bi_tu_choi() {
        for name in ["sqlite", "pg"] {
            assert!(error(&[name]).contains("is reserved"), "{name}");
        }
        // Chỉ đúng hai tên đó; tên chứa chúng thì không sao.
        assert!(Modules::new(bare(&["pgqueue", "sqlitex"])).is_ok());
    }

    #[test]
    fn dang_ky_trung_hoac_ten_long_nhau_bi_tu_choi() {
        assert!(error(&["auth", "auth"]).contains("module `auth` is registered twice"));
        assert!(error(&["auth", "auth_keys"]).contains("`auth` and `auth_keys` are ambiguous"));
        // `auth` và `authz`: tiền tố bảng là `auth_` và `authz_`, không đụng nhau.
        assert!(Modules::new(bare(&["auth", "authz"])).is_ok());
    }

    #[test]
    fn moi_loi_ten_duoc_bao_cung_luc() {
        let message = error(&["Auth", "notes", "notes"]);
        assert!(message.starts_with("modules: "), "{message}");
        assert!(message.contains("`Auth`") && message.contains("registered twice"), "{message}");
    }

    #[test]
    fn tung_module_doc_cau_hinh_cua_minh() {
        let source = ConfigSource::from_pairs([("AUTOXIDE_NOTES_LIMIT", "25")]);
        let mut reader = ConfigReader::new(&source);
        let mut notes = Notes::default();

        assert_eq!(notes.configure(&mut reader), Ok(()));

        assert_eq!(notes.limit, 25);
    }

    #[test]
    fn loi_cau_hinh_cua_moi_module_duoc_gom_chung() {
        // `notes` nhận giá trị sai, `tags` thiếu biến bắt buộc: cả hai phải có mặt
        // trong cùng một thông báo, dù `notes` đứng trước và đã hỏng.
        let source = ConfigSource::from_pairs([("AUTOXIDE_NOTES_LIMIT", "many")]);
        let mut reader = ConfigReader::new(&source);
        let mut modules =
            Modules::new(vec![Box::new(Notes::default()), Box::new(Tags), Box::new(Bare("auth"))])
                .unwrap();

        let configured = modules.configure(&mut reader);

        assert_eq!(configured, Err(Invalid));
        let message = reader.finish(configured).unwrap_err().to_string();
        assert!(message.contains("invalid configuration (2 problems):"), "{message}");
        assert!(message.contains("AUTOXIDE_NOTES_LIMIT:"), "{message}");
        assert!(message.contains("AUTOXIDE_TAGS_KEY: is required but not set"), "{message}");
    }

    #[test]
    fn module_khong_khai_gi_thi_khong_co_migration_va_cau_hinh() {
        let source = ConfigSource::default();
        let mut reader = ConfigReader::new(&source);
        let mut modules = Modules::new(bare(&["auth"])).unwrap();

        assert_eq!(modules.configure(&mut reader), Ok(()));
        assert!(reader.values().is_empty());
        assert!(modules.migrator().is_ok());
    }
}
