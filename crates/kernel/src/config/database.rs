//! Cấu hình database.

use std::fmt;
use std::str::FromStr;

use super::{ConfigReader, Invalid, Setting};

pub const DATABASE_URL: Setting = Setting::new(
    "DATABASE_URL",
    "Database to use. SQLite: `sqlite://path/to/file.sqlite?mode=rwc` (the file is created \
     when missing). PostgreSQL 16 or newer: `postgres://user:password@host:5432/database`.",
)
.with_default("sqlite://data/autoxide.sqlite?mode=rwc")
// URL của Postgres chứa mật khẩu.
.secret();

/// Hai database Autoxide hỗ trợ. Chỗ nào buộc phải rẽ nhánh theo database thì
/// `match` trên kiểu này: thêm một database mới là trình biên dịch chỉ ra mọi chỗ
/// phải xử lý.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseBackend {
    Sqlite,
    Postgres,
}

/// URL kết nối database đã kiểm là thuộc một database được hỗ trợ.
///
/// `Debug` che mật khẩu, nên kiểu này (và mọi struct chứa nó) in ra log an toàn.
/// Cần URL thật để kết nối thì gọi [`expose`](Self::expose).
#[derive(Clone, PartialEq, Eq)]
pub struct DatabaseUrl {
    url: String,
    backend: DatabaseBackend,
}

impl DatabaseUrl {
    pub fn backend(&self) -> DatabaseBackend {
        self.backend
    }

    /// URL đầy đủ, CÓ mật khẩu. Chỉ dùng để mở kết nối; không in ra, không ghi log.
    pub fn expose(&self) -> &str {
        &self.url
    }

    /// URL với mật khẩu thay bằng `***`, dùng để in ra.
    pub fn redacted(&self) -> String {
        redact_password(&self.url)
    }
}

impl FromStr for DatabaseUrl {
    // Lỗi là một câu cho người cài đặt đọc; `ConfigReader` ghép tên biến vào.
    type Err = String;

    fn from_str(url: &str) -> Result<Self, Self::Err> {
        let backend = if url.starts_with("sqlite:") {
            DatabaseBackend::Sqlite
        } else if url.starts_with("postgres://") || url.starts_with("postgresql://") {
            DatabaseBackend::Postgres
        } else {
            return Err("must start with `sqlite:` or `postgres://` — SQLite and PostgreSQL \
                        are the supported databases"
                .to_owned());
        };
        Ok(DatabaseUrl { url: url.to_owned(), backend })
    }
}

impl fmt::Debug for DatabaseUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DatabaseUrl({})", self.redacted())
    }
}

/// Thay mật khẩu trong một URL dạng `scheme://user:password@host/...` bằng `***`.
/// URL không có mật khẩu thì giữ nguyên.
pub fn redact_password(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_owned();
    };
    // Mật khẩu có thể chứa `@` nên lấy dấu `@` CUỐI trong phần authority (phần
    // trước dấu `/` đầu tiên).
    let authority_end = rest.find('/').unwrap_or(rest.len());
    let Some(at) = rest[..authority_end].rfind('@') else {
        return url.to_owned();
    };
    match rest[..at].split_once(':') {
        Some((user, _password)) => format!("{scheme}://{user}:***{}", &rest[at..]),
        None => url.to_owned(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseConfig {
    pub url: DatabaseUrl,
}

impl DatabaseConfig {
    pub fn read(reader: &mut ConfigReader) -> Result<Self, Invalid> {
        let url = reader.get(&DATABASE_URL);
        Ok(DatabaseConfig { url: url? })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigSource;

    fn read(pairs: &[(&str, &str)]) -> Result<DatabaseConfig, String> {
        let source = ConfigSource::from_pairs(pairs.iter().copied());
        let mut reader = ConfigReader::new(&source);
        let config = DatabaseConfig::read(&mut reader);
        reader.finish(config).map(|(config, _)| config).map_err(|errors| errors.to_string())
    }

    #[test]
    fn mac_dinh_la_sqlite_trong_thu_muc_data() {
        let config = read(&[]).unwrap();
        assert_eq!(config.url.backend(), DatabaseBackend::Sqlite);
        assert_eq!(config.url.expose(), "sqlite://data/autoxide.sqlite?mode=rwc");
    }

    #[test]
    fn nhan_sqlite_va_postgres() {
        for (url, backend) in [
            ("sqlite::memory:", DatabaseBackend::Sqlite),
            ("sqlite:///var/lib/autoxide/db.sqlite", DatabaseBackend::Sqlite),
            ("postgres://u:p@localhost:5432/app", DatabaseBackend::Postgres),
            ("postgresql://localhost/app", DatabaseBackend::Postgres),
        ] {
            assert_eq!(read(&[("DATABASE_URL", url)]).unwrap().url.backend(), backend, "{url}");
        }
    }

    #[test]
    fn database_khong_ho_tro_bi_tu_choi_va_khong_lo_url() {
        let message = read(&[("DATABASE_URL", "mysql://root:secret@localhost/app")]).unwrap_err();
        assert!(message.contains("DATABASE_URL: must start with `sqlite:` or `postgres://`"));
        assert!(!message.contains("secret"), "{message}");
    }

    #[test]
    fn debug_che_mat_khau() {
        let config = read(&[("DATABASE_URL", "postgres://app:s3cr3t@db:5432/app")]).unwrap();
        let printed = format!("{config:?}");
        assert!(printed.contains("postgres://app:***@db:5432/app"), "{printed}");
        assert!(!printed.contains("s3cr3t"), "{printed}");
    }

    #[test]
    fn che_mat_khau_trong_url() {
        assert_eq!(redact_password("postgres://u:secret@h:5432/db"), "postgres://u:***@h:5432/db");
        assert_eq!(redact_password("postgres://u:p@ss@h/db"), "postgres://u:***@h/db");
        // Không có mật khẩu thì giữ nguyên.
        assert_eq!(redact_password("postgres://u@h/db"), "postgres://u@h/db");
        assert_eq!(redact_password("postgres://h/db"), "postgres://h/db");
        assert_eq!(
            redact_password("sqlite://data/x.sqlite?mode=rwc"),
            "sqlite://data/x.sqlite?mode=rwc"
        );
        assert_eq!(redact_password("not a url"), "not a url");
    }
}
