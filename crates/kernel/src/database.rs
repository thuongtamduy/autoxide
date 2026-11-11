//! Kết nối database: tách kết nối ĐỌC và kết nối GHI.
//!
//! Vì sao tách (docs/DECISIONS.md D-009):
//!
//! - **SQLite là single-writer** — chỉ một lệnh ghi tại một thời điểm. Phía ghi
//!   vì thế dùng đúng MỘT kết nối: các lệnh ghi xếp hàng trong ứng dụng thay vì
//!   tranh lock và nhận lỗi "database is locked". Phía đọc dùng nhiều kết nối;
//!   nhờ WAL, đọc không bị lệnh ghi chặn.
//! - **Postgres** dùng cùng cách gọi: phía ghi là một pool tới database chính,
//!   phía đọc là một pool khác, trỏ được sang read replica.
//!
//! Kết nối đọc bị cấm ghi trên CẢ HAI database (SQLite mở read-only; Postgres đặt
//! `default_transaction_read_only`). Lỡ ghi qua [`Database::read`] thì hỏng ngay
//! ở test, trên database nào cũng vậy — chứ không lặng lẽ chạy được ở một bên.

use std::fmt;

use sea_orm::sqlx::sqlite::{SqliteJournalMode, SqliteSynchronous};
use sea_orm::{
    ConnectOptions, DatabaseConnection, DatabaseTransaction, DbErr, SqliteTransactionMode,
    TransactionOptions, TransactionTrait,
};

use crate::config::{DatabaseBackend, DatabaseConfig, DatabaseUrl};

/// Lỗi khi mở kết nối database.
#[derive(Debug)]
pub struct DatabaseError(String);

impl fmt::Display for DatabaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "database: {}", self.0)
    }
}

impl std::error::Error for DatabaseError {}

/// Hai pool kết nối tới database: một để đọc, một để ghi.
///
/// `Clone` rẻ: bên trong là con trỏ đếm tham chiếu tới pool, không mở thêm kết nối.
#[derive(Debug, Clone)]
pub struct Database {
    read: DatabaseConnection,
    write: DatabaseConnection,
    backend: DatabaseBackend,
}

impl Database {
    /// Mở hai pool theo `config`. Với SQLite: tạo thư mục và file nếu chưa có, bật
    /// WAL và foreign key.
    pub async fn connect(config: &DatabaseConfig) -> Result<Self, DatabaseError> {
        let backend = config.url.backend();

        if let Some(directory) = config.url.sqlite_path().as_deref().and_then(|path| path.parent())
        {
            // Gọi hệ thống file kiểu đồng bộ trong hàm async: chấp nhận được vì chỉ
            // chạy một lần lúc khởi động, trước khi phục vụ request nào.
            std::fs::create_dir_all(directory).map_err(|error| {
                DatabaseError(format!(
                    "cannot create directory `{}` for the SQLite file: {error}",
                    directory.display()
                ))
            })?;
        }

        // Mở phía ghi TRƯỚC: trên SQLite chính nó tạo file và chuyển file sang WAL;
        // phía đọc (read-only) không làm được hai việc đó.
        let write = open(write_options(config), &config.url).await?;
        let read_url = config.read_url.as_ref().unwrap_or(&config.url);
        let read = open(read_options(config, read_url), read_url).await?;

        Ok(Database { read, write, backend })
    }

    /// Kết nối cho lệnh ĐỌC. Ghi qua kết nối này bị database từ chối.
    pub fn read(&self) -> &DatabaseConnection {
        &self.read
    }

    /// Kết nối cho lệnh GHI. Một lệnh ghi đơn lẻ dùng thẳng kết nối này; nhiều
    /// lệnh phải cùng thành công hoặc cùng thất bại thì dùng
    /// [`begin_write`](Self::begin_write).
    pub fn write(&self) -> &DatabaseConnection {
        &self.write
    }

    /// Mở một transaction ghi. Gọi `.commit().await` để ghi nhận; transaction bị
    /// drop mà chưa commit thì tự rollback.
    ///
    /// Giữ transaction NGẮN: không gọi mạng, không hash mật khẩu bên trong — trên
    /// SQLite mọi lệnh ghi khác đang đứng chờ nó.
    ///
    /// Trên SQLite transaction mở bằng `BEGIN IMMEDIATE`: lấy quyền ghi ngay từ
    /// đầu. Kiểu mặc định (`DEFERRED`) chỉ xin quyền ghi khi gặp lệnh ghi đầu
    /// tiên, và nếu lúc đó một tiến trình khác vừa ghi thì transaction hỏng giữa
    /// chừng thay vì chờ.
    pub async fn begin_write(&self) -> Result<DatabaseTransaction, DbErr> {
        let options = TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            // `..Default::default()`: các trường còn lại lấy giá trị mặc định.
            ..Default::default()
        };
        self.write.begin_with_options(options).await
    }

    /// Database đang dùng là SQLite hay Postgres.
    pub fn backend(&self) -> DatabaseBackend {
        self.backend
    }

    /// Kiểm cả hai phía còn trả lời. Dùng cho health check.
    pub async fn ping(&self) -> Result<(), DbErr> {
        self.write.ping().await?;
        self.read.ping().await
    }

    /// Đóng cả hai pool, chờ các kết nối đóng hẳn. Gọi khi graceful shutdown.
    pub async fn close(self) -> Result<(), DbErr> {
        // Đóng phía đọc trước: trên SQLite, kết nối ĐÓNG CUỐI CÙNG là kết nối gộp
        // file WAL vào file chính, và việc đó cần quyền ghi.
        self.read.close().await?;
        self.write.close().await
    }
}

async fn open(
    options: ConnectOptions,
    url: &DatabaseUrl,
) -> Result<DatabaseConnection, DatabaseError> {
    sea_orm::Database::connect(options).await.map_err(|error| {
        // Thông báo dùng URL đã che mật khẩu; lỗi của driver không chứa mật khẩu.
        DatabaseError(format!("cannot connect to `{}`: {error}", url.redacted()))
    })
}

/// Phần chung của hai phía.
fn base_options(config: &DatabaseConfig, url: &DatabaseUrl) -> ConnectOptions {
    let mut options = ConnectOptions::new(url.expose().to_owned());
    options
        .acquire_timeout(config.wait)
        // Log câu lệnh SQL do mục "Log, metrics, tracing" của lộ trình quyết định;
        // mặc định của thư viện là in mọi câu lệnh ở mức INFO.
        .sqlx_logging(false);
    options
}

fn write_options(config: &DatabaseConfig) -> ConnectOptions {
    let mut options = base_options(config, &config.url);
    let wait = config.wait;
    match config.url.backend() {
        DatabaseBackend::Sqlite => {
            options
                // Đúng một kết nối ghi, giữ mở suốt đời tiến trình.
                .max_connections(1)
                .min_connections(1)
                .map_sqlx_sqlite_opts(move |sqlite| {
                    sqlite
                        .create_if_missing(true)
                        // WAL: đọc và ghi không chặn nhau. Chế độ này được lưu
                        // trong file nên kết nối đọc tự thấy.
                        .journal_mode(SqliteJournalMode::Wal)
                        // NORMAL là mức khuyến nghị cho WAL: an toàn trước việc
                        // ứng dụng sập; mất điện có thể mất vài transaction cuối
                        // nhưng không hỏng file.
                        .synchronous(SqliteSynchronous::Normal)
                        // SQLite KHÔNG kiểm foreign key nếu không bật, mỗi kết nối.
                        .foreign_keys(true)
                        // Chờ lock của tiến trình khác (vd api và worker chạy riêng)
                        // thay vì báo lỗi ngay.
                        .busy_timeout(wait)
                });
        }
        DatabaseBackend::Postgres => {
            options
                .max_connections(config.write_connections)
                .map_sqlx_postgres_opts(|postgres| postgres.application_name("autoxide"));
        }
    }
    options
}

fn read_options(config: &DatabaseConfig, url: &DatabaseUrl) -> ConnectOptions {
    let mut options = base_options(config, url);
    options.max_connections(config.read_connections);
    let wait = config.wait;
    match url.backend() {
        DatabaseBackend::Sqlite => {
            options.map_sqlx_sqlite_opts(move |sqlite| {
                sqlite.read_only(true).foreign_keys(true).busy_timeout(wait)
            });
        }
        DatabaseBackend::Postgres => {
            options.map_sqlx_postgres_opts(|postgres| {
                postgres
                    .application_name("autoxide-read")
                    // Mọi transaction trên kết nối này mặc định chỉ đọc.
                    .options([("default_transaction_read_only", "on")])
            });
        }
    }
    options
}
