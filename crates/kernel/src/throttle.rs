//! Giới hạn số lần theo khoá, đếm trong database (D-110): đúng khi chạy nhiều máy chủ và khi
//! nhiều request cùng lúc, trên cả SQLite và Postgres.
//!
//! Dùng cho: khoá đăng nhập theo email (`auth`), giới hạn gửi form theo IP băm hay theo email
//! (chống spam), giới hạn email xác nhận gửi tới một địa chỉ…
//!
//! - Module thêm `throttle_migration("m00xx_create_throttle", "<module>")` vào migration của
//!   mình (bảng `<module>_throttle`), như `jobs_migration`.
//! - Mỗi khoá có một cửa sổ đếm dài `window`. Vượt `limit` lần trong cửa sổ thì khoá bị chặn
//!   trong `lock` (mặc định bằng `window`), rồi đếm lại từ đầu.
//! - Khoá đếm được băm SHA-256 trước khi lưu: email hay IP không nằm nguyên văn trong bảng.
//! - Mỗi bước là MỘT câu `UPDATE` có điều kiện — database tự xếp hàng các request đồng thời,
//!   không request nào ghi đè số đếm của request khác. [`Throttle::hit`] đếm TRƯỚC rồi mới
//!   kiểm, nên N request bắn cùng lúc không vượt được `limit`.

use std::time::Duration;

use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Condition, Expr, ExprTrait as _, OnConflict, Query};
use sea_orm_migration::prelude::{
    DbErr, Index, MigrationName, MigrationTrait, SchemaManager, Table,
};
use sea_orm_migration::schema;

use crate::database::Database;
use crate::error::Error;
use crate::secret::sha256;

/// Kết quả của một lần xét.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Allowed,
    /// Bị chặn; thử lại sau chừng này giây (làm tròn lên).
    Limited {
        retry_after: u64,
    },
}

impl Outcome {
    pub fn is_allowed(self) -> bool {
        self == Outcome::Allowed
    }
}

/// Bộ đếm trên bảng `<module>_throttle`.
#[derive(Clone)]
pub struct Throttle {
    database: Database,
    table: String,
    limit: i32,
    window: chrono::Duration,
    lock: chrono::Duration,
}

impl std::fmt::Debug for Throttle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Throttle")
            .field("table", &self.table)
            .field("limit", &self.limit)
            .finish_non_exhaustive()
    }
}

impl Throttle {
    /// Cho phép `limit` lần mỗi `window` trên bảng `<module>_throttle`.
    pub fn new(
        database: Database,
        module: &str,
        limit: u32,
        window: Duration,
    ) -> Result<Self, Error> {
        let limit = <i32 as TryFrom<u32>>::try_from(limit).map_err(Error::internal)?;
        let window = chrono::Duration::from_std(window).map_err(Error::internal)?;
        Ok(Throttle { database, table: table_name(module), limit, window, lock: window })
    }

    /// Vượt giới hạn thì chặn trong `lock` (thay vì `window`).
    pub fn with_lock(mut self, lock: Duration) -> Result<Self, Error> {
        self.lock = chrono::Duration::from_std(lock).map_err(Error::internal)?;
        Ok(self)
    }

    /// Khoá `key` có đang bị chặn không, KHÔNG đếm.
    pub async fn check(&self, key: &str) -> Result<Outcome, Error> {
        let now = now();
        let select = Query::select()
            .column(Alias::new("locked_until"))
            .from(Alias::new(&self.table))
            .and_where(Expr::col(Alias::new("key")).eq(sha256(key)))
            .to_owned();
        let row = self.database.write().query_one(&select).await?;
        let until = match row {
            Some(row) => row.try_get::<Option<DateTime<Utc>>>("", "locked_until")?,
            None => None,
        };
        Ok(match until {
            Some(until) if until > now => {
                let wait = Ord::max((until - now).num_milliseconds(), 0);
                Outcome::Limited {
                    retry_after: Ord::max(
                        <u64 as TryFrom<i64>>::try_from((wait + 999) / 1000).unwrap_or(1),
                        1,
                    ),
                }
            }
            _ => Outcome::Allowed,
        })
    }

    /// Đếm một lần cho `key` rồi xét: bị chặn (từ trước hoặc vì lần này vượt giới hạn) thì
    /// [`Outcome::Limited`]. Lần đang bị chặn thì không đếm thêm.
    pub async fn hit(&self, key: &str) -> Result<Outcome, Error> {
        let before = self.check(key).await?;
        if !before.is_allowed() {
            return Ok(before);
        }
        self.record(key).await?;
        self.check(key).await
    }

    /// Đếm một lần cho `key`, không xét (vd ghi một lần đăng nhập sai).
    pub async fn record(&self, key: &str) -> Result<(), Error> {
        let hashed = sha256(key);
        let now = now();
        let window_start = now - self.window;
        let table = Alias::new(&self.table);
        let connection = self.database.write();
        // Vòng lặp chỉ chạy lại một lần, khi hai request cùng chèn dòng đầu tiên.
        for _ in 0..2 {
            // 1. Đang trong cửa sổ đếm: cộng một.
            let counted = Query::update()
                .table(table.clone())
                .value(Alias::new("count"), Expr::col(Alias::new("count")).add(1))
                .and_where(Expr::col(Alias::new("key")).eq(hashed.as_str()))
                .and_where(Expr::col(Alias::new("window_started_at")).gt(window_start))
                .to_owned();
            if connection.execute(&counted).await?.rows_affected() > 0 {
                return self.lock_if_exceeded(&hashed, now).await;
            }
            // 2. Cửa sổ cũ đã hết: bắt đầu cửa sổ mới với một lần.
            let restarted = Query::update()
                .table(table.clone())
                .value(Alias::new("count"), 1)
                .value(Alias::new("window_started_at"), now)
                .and_where(Expr::col(Alias::new("key")).eq(hashed.as_str()))
                .and_where(Expr::col(Alias::new("window_started_at")).lte(window_start))
                .to_owned();
            if connection.execute(&restarted).await?.rows_affected() > 0 {
                return self.lock_if_exceeded(&hashed, now).await;
            }
            // 3. Chưa có dòng: chèn. Request khác vừa chèn trước thì quay lại bước 1.
            let inserted = Query::insert()
                .into_table(table.clone())
                .columns([Alias::new("key"), Alias::new("count"), Alias::new("window_started_at")])
                .values_panic([hashed.as_str().into(), 1.into(), now.into()])
                .on_conflict(OnConflict::column(Alias::new("key")).do_nothing().to_owned())
                .to_owned();
            if connection.execute(&inserted).await?.rows_affected() > 0 {
                return self.lock_if_exceeded(&hashed, now).await;
            }
        }
        Ok(())
    }

    /// Trả lại một lần đã đếm (vd lần thử hoá ra không sai): người dùng thật không bị tính oan.
    pub async fn refund(&self, key: &str) -> Result<(), Error> {
        let update = Query::update()
            .table(Alias::new(&self.table))
            .value(Alias::new("count"), Expr::col(Alias::new("count")).sub(1))
            .and_where(Expr::col(Alias::new("key")).eq(sha256(key)))
            .and_where(Expr::col(Alias::new("count")).gt(0))
            .to_owned();
        self.database.write().execute(&update).await?;
        Ok(())
    }

    /// Quên mọi lần đã đếm của `key` (vd đăng nhập đúng).
    pub async fn clear(&self, key: &str) -> Result<(), Error> {
        let delete = Query::delete()
            .from_table(Alias::new(&self.table))
            .and_where(Expr::col(Alias::new("key")).eq(sha256(key)))
            .to_owned();
        self.database.write().execute(&delete).await?;
        Ok(())
    }

    /// Xoá các dòng có cửa sổ đã qua trước `before` và không còn bị chặn. Module gọi trong job
    /// dọn dẹp định kỳ của mình. Trả số dòng đã xoá.
    pub async fn purge(&self, before: DateTime<Utc>) -> Result<u64, Error> {
        let now = now();
        let delete = Query::delete()
            .from_table(Alias::new(&self.table))
            .cond_where(
                Condition::all().add(Expr::col(Alias::new("window_started_at")).lt(before)).add(
                    Condition::any()
                        .add(Expr::col(Alias::new("locked_until")).is_null())
                        .add(Expr::col(Alias::new("locked_until")).lt(now)),
                ),
            )
            .to_owned();
        Ok(self.database.write().execute(&delete).await?.rows_affected())
    }

    /// Vượt ngưỡng thì chặn tới `now + lock` và đếm lại từ đầu, để hết chặn là có đủ số lần
    /// như mới.
    async fn lock_if_exceeded(&self, hashed: &str, now: DateTime<Utc>) -> Result<(), Error> {
        let update = Query::update()
            .table(Alias::new(&self.table))
            .value(Alias::new("locked_until"), Some(now + self.lock))
            .value(Alias::new("count"), 0)
            .value(Alias::new("window_started_at"), now)
            .and_where(Expr::col(Alias::new("key")).eq(hashed))
            // Lượt vượt mức mới chặn: `limit` lượt đầu vẫn được.
            .and_where(Expr::col(Alias::new("count")).gt(self.limit))
            .to_owned();
        let locked = self.database.write().execute(&update).await?.rows_affected();
        if locked > 0 {
            metrics::counter!("kernel_throttle_locks_total", "table" => self.table.clone())
                .increment(1);
        }
        Ok(())
    }
}

fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

fn table_name(module: &str) -> String {
    format!("{module}_throttle")
}

/// Migration tạo bảng `<module>_throttle`. Module dùng [`Throttle`] thêm nó vào danh sách
/// migration của mình.
pub fn throttle_migration(name: &'static str, module: &'static str) -> Box<dyn MigrationTrait> {
    Box::new(ThrottleMigration { name, module })
}

struct ThrottleMigration {
    name: &'static str,
    module: &'static str,
}

impl MigrationName for ThrottleMigration {
    fn name(&self) -> &str {
        self.name
    }
}

#[async_trait::async_trait]
impl MigrationTrait for ThrottleMigration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let table = table_name(self.module);
        manager
            .create_table(
                Table::create()
                    .table(Alias::new(&table))
                    // SHA-256 (base64url) của khoá đếm.
                    .col(schema::string_len("key", 64).primary_key())
                    .col(schema::integer("count"))
                    .col(schema::timestamp_with_time_zone("window_started_at"))
                    .col(schema::timestamp_with_time_zone_null("locked_until"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name(format!("{table}_window_idx"))
                    .table(Alias::new(&table))
                    .col("window_started_at")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Alias::new(table_name(self.module))).to_owned())
            .await
    }
}
