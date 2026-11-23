//! Database configuration.

use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use super::{ConfigReader, Invalid, Setting, parse_in_range};

pub const DATABASE_URL: Setting = Setting::new(
    "DATABASE_URL",
    "Database to use. SQLite: `sqlite://path/to/file.sqlite` (the file and its directory are \
     created when missing). PostgreSQL 16 or newer: `postgres://user:password@host:5432/database`.",
)
.with_default("sqlite://data/autoxide.sqlite")
// URL của Postgres chứa mật khẩu.
.secret();

pub const DATABASE_READ_URL: Setting = Setting::new(
    "DATABASE_READ_URL",
    "PostgreSQL only: a read replica to send read queries to. When unset, reads use DATABASE_URL.",
)
.with_example("postgres://user:password@replica:5432/database")
.secret();

pub const WRITE_CONNECTIONS: Setting = Setting::new(
    "AUTOXIDE_DATABASE_WRITE_CONNECTIONS",
    "Maximum number of connections used for writes. SQLite always uses exactly one, \
     because SQLite allows a single writer at a time.",
)
.with_default("10");

pub const READ_CONNECTIONS: Setting = Setting::new(
    "AUTOXIDE_DATABASE_READ_CONNECTIONS",
    "Maximum number of connections used for read queries.",
)
.with_default("10");

pub const WAIT_SECONDS: Setting = Setting::new(
    "AUTOXIDE_DATABASE_WAIT_SECONDS",
    "How long a query waits for a free connection, or for a lock held by another writer, \
     before failing.",
)
.with_default("10");

/// The two databases Autoxide supports. Where code must branch on the database, it uses `match` on this
/// type: adding a new database makes the compiler point out every place that needs handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseBackend {
    Sqlite,
    Postgres,
}

/// A database connection URL, checked to belong to a supported database.
///
/// `Debug` masks the password, so this type (and every struct containing it) is safe to print in logs. To
/// get the real URL for connecting, call [`expose`](Self::expose).
#[derive(Clone, PartialEq, Eq)]
pub struct DatabaseUrl {
    url: String,
    backend: DatabaseBackend,
}

impl DatabaseUrl {
    pub fn backend(&self) -> DatabaseBackend {
        self.backend
    }

    /// The full URL, WITH the password. Use only to open connections; never print or log it.
    pub fn expose(&self) -> &str {
        &self.url
    }

    /// The URL with the password replaced by `***`, for printing.
    pub fn redacted(&self) -> String {
        redact_password(&self.url)
    }

    /// The file path of a SQLite URL; `None` for Postgres and for in-memory SQLite.
    pub fn sqlite_path(&self) -> Option<PathBuf> {
        if self.backend != DatabaseBackend::Sqlite || self.is_sqlite_in_memory() {
            return None;
        }
        Some(PathBuf::from(sqlite_location(&self.url).0))
    }

    /// Whether the URL points to an in-memory SQLite database (`sqlite::memory:`, `?mode=memory`)?
    fn is_sqlite_in_memory(&self) -> bool {
        let (path, query) = sqlite_location(&self.url);
        self.backend == DatabaseBackend::Sqlite
            && (path.is_empty()
                || path == ":memory:"
                || query.split('&').any(|p| p == "mode=memory"))
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

/// Splits a SQLite URL into (file path, query string). Accepts all three forms: `sqlite:file`,
/// `sqlite://file` and `sqlite:///abs/file`.
fn sqlite_location(url: &str) -> (&str, &str) {
    let rest = url.strip_prefix("sqlite:").unwrap_or(url);
    let rest = rest.strip_prefix("//").unwrap_or(rest);
    rest.split_once('?').unwrap_or((rest, ""))
}

/// Replaces the password in a URL of the form `scheme://user:password@host/...` with `***`. A URL without a
/// password is kept as is.
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
#[non_exhaustive]
pub struct DatabaseConfig {
    /// The primary database: every write goes here.
    pub url: DatabaseUrl,
    /// The Postgres read replica for reads; with `None`, reads come from `url`.
    pub read_url: Option<DatabaseUrl>,
    /// The maximum number of write connections (Postgres). SQLite always uses one.
    pub write_connections: u32,
    /// The maximum number of read connections.
    pub read_connections: u32,
    /// How long to wait for an idle connection, or for the lock held by another writer.
    pub wait: Duration,
}

impl DatabaseConfig {
    pub fn read(reader: &mut ConfigReader) -> Result<Self, Invalid> {
        // Đọc hết trước, dùng `?` sau: một biến sai không chặn việc kiểm biến khác.
        let url = reader.get::<DatabaseUrl>(&DATABASE_URL);
        let read_url = reader.optional::<DatabaseUrl>(&DATABASE_READ_URL);
        let write_connections =
            reader.get_with(&WRITE_CONNECTIONS, |raw| parse_in_range(raw, 1u32, 1000));
        let read_connections =
            reader.get_with(&READ_CONNECTIONS, |raw| parse_in_range(raw, 1u32, 1000));
        let wait = reader.get_with(&WAIT_SECONDS, |raw| parse_in_range(raw, 1u64, 300));
        let (url, read_url) = (url?, read_url?);

        if url.is_sqlite_in_memory() {
            return reader.reject(
                &DATABASE_URL,
                "in-memory SQLite is not supported: every connection would see its own empty \
                 database. Use a file, for example `sqlite://data/autoxide.sqlite`",
            );
        }
        if let Some(read_url) = &read_url {
            let both_postgres = url.backend() == DatabaseBackend::Postgres
                && read_url.backend() == DatabaseBackend::Postgres;
            if !both_postgres {
                return reader.reject(
                    &DATABASE_READ_URL,
                    "is only for a PostgreSQL read replica: both DATABASE_URL and \
                     DATABASE_READ_URL must be `postgres://` URLs. Unset it when using SQLite",
                );
            }
        }

        Ok(DatabaseConfig {
            url,
            read_url,
            write_connections: write_connections?,
            read_connections: read_connections?,
            wait: Duration::from_secs(wait?),
        })
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
        assert_eq!(config.url.expose(), "sqlite://data/autoxide.sqlite");
        assert_eq!(config.read_url, None);
        assert_eq!((config.write_connections, config.read_connections), (10, 10));
        assert_eq!(config.wait, Duration::from_secs(10));
    }

    #[test]
    fn duong_dan_file_sqlite_doc_duoc_tu_moi_dang_url() {
        for (url, path) in [
            ("sqlite://data/autoxide.sqlite", "data/autoxide.sqlite"),
            ("sqlite://data/autoxide.sqlite?mode=rwc", "data/autoxide.sqlite"),
            ("sqlite:data/autoxide.sqlite", "data/autoxide.sqlite"),
            ("sqlite:///var/lib/autoxide/db.sqlite", "/var/lib/autoxide/db.sqlite"),
        ] {
            let config = read(&[("DATABASE_URL", url)]).unwrap();
            assert_eq!(config.url.sqlite_path(), Some(PathBuf::from(path)), "{url}");
        }
        let postgres = read(&[("DATABASE_URL", "postgres://h/db")]).unwrap();
        assert_eq!(postgres.url.sqlite_path(), None);
    }

    #[test]
    fn sqlite_trong_bo_nho_bi_tu_choi() {
        for url in ["sqlite::memory:", "sqlite://:memory:", "sqlite:", "sqlite://x?mode=memory"] {
            let message = read(&[("DATABASE_URL", url)]).unwrap_err();
            assert!(
                message.contains("DATABASE_URL: in-memory SQLite is not supported"),
                "{url}: {message}"
            );
        }
    }

    #[test]
    fn read_replica_chi_danh_cho_postgres() {
        let primary = ("DATABASE_URL", "postgres://app:pw@db/app");
        let replica = ("DATABASE_READ_URL", "postgres://app:pw@replica/app");
        let config = read(&[primary, replica]).unwrap();
        assert_eq!(config.read_url.unwrap().expose(), "postgres://app:pw@replica/app");

        for pairs in [
            // SQLite không có read replica.
            vec![replica],
            vec![("DATABASE_URL", "sqlite://a.sqlite"), replica],
            // Replica phải cùng loại database.
            vec![primary, ("DATABASE_READ_URL", "sqlite://b.sqlite")],
        ] {
            let message = read(&pairs).unwrap_err();
            assert!(
                message.contains("DATABASE_READ_URL: is only for a PostgreSQL read replica"),
                "{message}"
            );
            assert!(!message.contains("pw@"), "{message}");
        }
    }

    #[test]
    fn so_ket_noi_va_thoi_gian_cho_phai_trong_khoang() {
        let config = read(&[
            ("AUTOXIDE_DATABASE_WRITE_CONNECTIONS", "3"),
            ("AUTOXIDE_DATABASE_READ_CONNECTIONS", "20"),
            ("AUTOXIDE_DATABASE_WAIT_SECONDS", "2"),
        ])
        .unwrap();
        assert_eq!((config.write_connections, config.read_connections), (3, 20));
        assert_eq!(config.wait, Duration::from_secs(2));

        let message = read(&[
            ("AUTOXIDE_DATABASE_WRITE_CONNECTIONS", "0"),
            ("AUTOXIDE_DATABASE_READ_CONNECTIONS", "many"),
            ("AUTOXIDE_DATABASE_WAIT_SECONDS", "301"),
        ])
        .unwrap_err();
        assert!(message.contains("invalid configuration (3 problems):"), "{message}");
        assert!(message.contains(
            "AUTOXIDE_DATABASE_WRITE_CONNECTIONS: must be a whole number between 1 and 1000 (got `0`)"
        ));
        assert!(
            message.contains(
                "AUTOXIDE_DATABASE_WAIT_SECONDS: must be a whole number between 1 and 300"
            )
        );
    }

    #[test]
    fn nhan_sqlite_va_postgres() {
        for (url, backend) in [
            ("sqlite://data/app.sqlite", DatabaseBackend::Sqlite),
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
