//! Where configuration values come from: the process environment variables and the `.env` file.

use std::collections::BTreeMap;
use std::path::Path;

use super::{ConfigError, ConfigErrors};

/// The set of raw configuration values (name to string), merged from every source.
///
/// Configuration-reading code does not call `std::env::var` directly but goes through this type, so tests
/// can pass values in with [`from_pairs`](Self::from_pairs) without modifying the process environment
/// (which is `unsafe` since Rust 2024 because other threads may be reading it).
#[derive(Debug, Clone, Default)]
pub struct ConfigSource {
    values: BTreeMap<String, String>,
}

impl ConfigSource {
    /// Builds from the given (name, value) pairs.
    pub fn from_pairs<I, K, V>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        ConfigSource {
            values: pairs.into_iter().map(|(name, value)| (name.into(), value.into())).collect(),
        }
    }

    /// The process environment variables, plus the `.env` file at `dotenv_path` if the file exists. Real
    /// environment variables WIN over the `.env` file, so Docker, systemd or CI can override without
    /// editing the file.
    pub fn load(dotenv_path: &Path) -> Result<Self, ConfigErrors> {
        let mut values = BTreeMap::new();

        if dotenv_path.exists() {
            let file = std::fs::File::open(dotenv_path).map_err(|error| {
                ConfigErrors::single(ConfigError::file(dotenv_path, error.to_string()))
            })?;
            for pair in dotenvy::from_read_iter(file) {
                let (name, value) = pair.map_err(|error| {
                    ConfigErrors::single(ConfigError::file(dotenv_path, error.to_string()))
                })?;
                values.insert(name, value);
            }
        }

        // `vars_os` thay cho `vars`: `vars` panic khi gặp biến không phải UTF-8.
        // Biến như vậy không thể là cấu hình của Autoxide nên bỏ qua.
        for (name, value) in std::env::vars_os() {
            if let (Ok(name), Ok(value)) = (name.into_string(), value.into_string()) {
                values.insert(name, value);
            }
        }

        Ok(ConfigSource { values })
    }

    /// The value of `name`, trimmed of surrounding whitespace. An empty string counts as unset: `VAR=` in a
    /// `.env` file or compose means "use the default".
    pub fn get(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(|value| value.trim()).filter(|value| !value.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chuoi_rong_va_khoang_trang_coi_nhu_khong_dat() {
        let source = ConfigSource::from_pairs([("A", ""), ("B", "   "), ("C", " x ")]);
        assert_eq!(source.get("A"), None);
        assert_eq!(source.get("B"), None);
        assert_eq!(source.get("C"), Some("x"));
        assert_eq!(source.get("MISSING"), None);
    }

    /// A temporary directory private to one test, removed automatically when done.
    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("autoxide-config-{label}-{}", std::process::id()));
            std::fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn doc_file_dotenv_va_bien_moi_truong_that_thang() {
        let directory = TempDir::new("dotenv");
        let dotenv = directory.0.join(".env");
        // PATH luôn có trong môi trường của tiến trình test: dùng nó để kiểm thứ
        // tự ưu tiên mà không phải sửa môi trường.
        std::fs::write(
            &dotenv,
            "# chú thích\nAUTOXIDE_TEST_ONLY_IN_FILE=from-file\nQUOTED=\"a b\"\nPATH=from-file\n",
        )
        .unwrap();

        let source = ConfigSource::load(&dotenv).unwrap();

        assert_eq!(source.get("AUTOXIDE_TEST_ONLY_IN_FILE"), Some("from-file"));
        assert_eq!(source.get("QUOTED"), Some("a b"));
        assert_ne!(source.get("PATH"), Some("from-file"));
        assert!(source.get("PATH").is_some());
    }

    #[test]
    fn khong_co_file_dotenv_thi_chi_dung_moi_truong() {
        let directory = TempDir::new("missing");
        let source = ConfigSource::load(&directory.0.join(".env")).unwrap();
        assert!(source.get("PATH").is_some());
    }

    #[test]
    fn file_dotenv_sai_cu_phap_la_loi_co_ten_file() {
        let directory = TempDir::new("broken");
        let dotenv = directory.0.join(".env");
        std::fs::write(&dotenv, "THIS LINE IS NOT VALID\n").unwrap();

        let message = ConfigSource::load(&dotenv).unwrap_err().to_string();

        assert!(message.contains(".env"), "{message}");
    }
}
