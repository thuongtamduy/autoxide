//! Giới hạn số lần theo khoá, đếm trong database (D-116): đúng khi chạy nhiều máy chủ và khi
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
//! - Mỗi lần đếm là MỘT câu "chèn hoặc cộng" trả về số đếm (D-321) — database tự xếp hàng các
//!   request đồng thời, không request nào ghi đè số đếm của request khác. [`Throttle::hit`] đếm
//!   TRƯỚC rồi mới kiểm, nên N request bắn cùng lúc không vượt được `limit`.
//! - Theo khách (bước 33): mỗi hàm nhận khách; số đếm nằm ở database của khách, khoá chính
//!   `(tenant_id, key)` — một khách bị dồn không làm khoá người dùng của khách khác.

use std::time::Duration;

use crate::migration::prelude::{
    DbErr, Index, MigrationName, MigrationTrait, SchemaManager, Table,
};
use crate::migration::schema;
use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Condition, Expr, ExprTrait as _, OnConflict, Query};

use crate::database::Database;
use crate::error::Error;
use crate::secret::sha256;
use crate::tenancy::TenantContext;

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
    pub fn new(module: &str, limit: u32, window: Duration) -> Result<Self, Error> {
        let limit = <i32 as TryFrom<u32>>::try_from(limit).map_err(Error::internal)?;
        let window = chrono::Duration::from_std(window).map_err(Error::internal)?;
        Ok(Throttle { table: table_name(module), limit, window, lock: window })
    }

    /// Module sở hữu bảng (`<module>_throttle`) — để chống spam đọc bảng nonce của cùng module.
    pub(crate) fn owner(&self) -> Option<&str> {
        self.table.strip_suffix("_throttle")
    }

    /// Vượt giới hạn thì chặn trong `lock` (thay vì `window`).
    pub fn with_lock(mut self, lock: Duration) -> Result<Self, Error> {
        self.lock = chrono::Duration::from_std(lock).map_err(Error::internal)?;
        Ok(self)
    }

    /// Khoá `key` của khách `tenant` có đang bị chặn không, KHÔNG đếm.
    pub async fn check(&self, tenant: &TenantContext, key: &str) -> Result<Outcome, Error> {
        let now = now();
        let select = Query::select()
            .column(Alias::new("locked_until"))
            .from(Alias::new(&self.table))
            .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
            .and_where(Expr::col(Alias::new("key")).eq(sha256(key)))
            .to_owned();
        let row = tenant.database().write().query_one(&select).await?;
        let until = match row {
            Some(row) => row.try_get::<Option<DateTime<Utc>>>("", "locked_until")?,
            None => None,
        };
        Ok(match until {
            Some(until) if until > now => Outcome::Limited { retry_after: seconds_up(until - now) },
            _ => Outcome::Allowed,
        })
    }

    /// Đếm một lần cho `key` rồi xét: bị chặn (từ trước hoặc vì lần này vượt giới hạn) thì
    /// [`Outcome::Limited`]. Lần đang bị chặn thì không đếm thêm.
    ///
    /// Hai câu cho lượt được phép (D-321): đọc xem có đang bị chặn không (không khoá dòng — lượt bị
    /// chặn dừng ở đây), rồi một câu "chèn hoặc cộng" trả về số đếm của CHÍNH lượt này. Mỗi lượt đồng
    /// thời nhận một số đếm khác nhau, nên đúng `limit` lượt được phép.
    pub async fn hit(&self, tenant: &TenantContext, key: &str) -> Result<Outcome, Error> {
        let before = self.check(tenant, key).await?;
        if !before.is_allowed() {
            return Ok(before);
        }
        let hashed = sha256(key);
        let now = now();
        let counted = self.count(tenant, &hashed, now).await?;
        // Một lượt đồng thời khác vừa khoá (và đặt lại số đếm về 0) sau khi lượt này qua bước
        // `check`: số đếm trả về lại nhỏ, nhưng khoá đang có hiệu lực — vẫn phải chặn.
        if let Some(until) = counted.locked_until.filter(|until| *until > now) {
            return Ok(Outcome::Limited { retry_after: seconds_up(until - now) });
        }
        if counted.count > i64::from(self.limit) {
            self.lock_if_exceeded(tenant, &hashed, now).await?;
            return Ok(Outcome::Limited { retry_after: seconds_up(self.lock) });
        }
        Ok(Outcome::Allowed)
    }

    /// Đếm một lần cho `key`, không xét (vd ghi một lần đăng nhập sai).
    pub async fn record(&self, tenant: &TenantContext, key: &str) -> Result<(), Error> {
        let hashed = sha256(key);
        let now = now();
        if self.count(tenant, &hashed, now).await?.count > i64::from(self.limit) {
            self.lock_if_exceeded(tenant, &hashed, now).await?;
        }
        Ok(())
    }

    /// Cộng một vào số đếm của khoá đã băm `hashed` bằng MỘT câu và trả số đếm sau khi cộng: chưa
    /// có dòng thì chèn với 1; cửa sổ cũ đã hết thì bắt đầu cửa sổ mới với 1. `INSERT … ON CONFLICT
    /// DO UPDATE … RETURNING` chạy trên cả SQLite (từ 3.35) và Postgres; database tự xếp hàng các
    /// lượt cùng lúc trên dòng đó. Trả kèm `locked_until` của dòng (câu cộng không đổi cột này).
    async fn count(
        &self,
        tenant: &TenantContext,
        hashed: &str,
        now: DateTime<Utc>,
    ) -> Result<Counted, Error> {
        let table = Alias::new(&self.table);
        // Cột của dòng đang có, ghi kèm tên bảng cho rõ (không lẫn với `excluded`).
        let column = |name: &str| Expr::col((table.clone(), Alias::new(name)));
        let fresh = column("window_started_at").gt(now - self.window);
        let upsert = Query::insert()
            .into_table(table.clone())
            .columns(["tenant_id", "key", "count", "window_started_at"].map(Alias::new))
            .values_panic([tenant.id().as_uuid().into(), hashed.into(), 1.into(), now.into()])
            .on_conflict(
                OnConflict::columns([Alias::new("tenant_id"), Alias::new("key")])
                    .value(
                        Alias::new("count"),
                        Expr::case(fresh.clone(), column("count").add(1)).finally(1),
                    )
                    .value(
                        Alias::new("window_started_at"),
                        Expr::case(fresh, column("window_started_at")).finally(now),
                    )
                    .to_owned(),
            )
            .returning(Query::returning().columns(["count", "locked_until"].map(Alias::new)))
            .to_owned();
        let row = tenant.database().write().query_one(&upsert).await?.ok_or_else(|| {
            Error::internal(format!("counting in `{}` returned no row", self.table))
        })?;
        Ok(Counted {
            count: i64::from(row.try_get::<i32>("", "count")?),
            locked_until: row.try_get::<Option<DateTime<Utc>>>("", "locked_until")?,
        })
    }

    /// Lấy tối đa `n` lượt của khoá `key` trong cửa sổ hiện tại, bằng MỘT câu (D-375): dùng cho
    /// rate limit theo lô — mỗi máy chủ lấy một lô rồi tiêu dần trong bộ nhớ, hết mới hỏi lại. Số
    /// đếm cộng thẳng `n` (vượt `limit` cũng không sao: lượt sau trong cửa sổ nhận 0), nên số lượt
    /// được cấp tính được từ số đếm trả về: `min(n, limit − (count − n))`. Các lượt đồng thời được
    /// database xếp hàng trên dòng, tổng số lượt cấp trong một cửa sổ không vượt `limit`. Không đụng
    /// tới `locked_until` (của [`hit`](Self::hit)).
    pub async fn take(&self, tenant: &TenantContext, key: &str, n: u32) -> Result<Taken, Error> {
        let wanted = <i32 as TryFrom<u32>>::try_from(n).map_err(Error::internal)?;
        let now = now();
        let table = Alias::new(&self.table);
        let column = |name: &str| Expr::col((table.clone(), Alias::new(name)));
        let fresh = column("window_started_at").gt(now - self.window);
        let upsert = Query::insert()
            .into_table(table.clone())
            .columns(["tenant_id", "key", "count", "window_started_at"].map(Alias::new))
            .values_panic([
                tenant.id().as_uuid().into(),
                sha256(key).into(),
                wanted.into(),
                now.into(),
            ])
            .on_conflict(
                OnConflict::columns([Alias::new("tenant_id"), Alias::new("key")])
                    .value(
                        Alias::new("count"),
                        Expr::case(fresh.clone(), column("count").add(wanted)).finally(wanted),
                    )
                    .value(
                        Alias::new("window_started_at"),
                        Expr::case(fresh, column("window_started_at")).finally(now),
                    )
                    .to_owned(),
            )
            .returning(Query::returning().columns(["count", "window_started_at"].map(Alias::new)))
            .to_owned();
        let row = tenant.database().write().query_one(&upsert).await?.ok_or_else(|| {
            Error::internal(format!("taking from `{}` returned no row", self.table))
        })?;
        let count = i64::from(row.try_get::<i32>("", "count")?);
        let started = row.try_get::<DateTime<Utc>>("", "window_started_at")?;
        let before = count - i64::from(wanted);
        let granted = (i64::from(self.limit) - before).clamp(0, i64::from(wanted));
        let ends = started + self.window;
        Ok(Taken {
            granted: <u32 as TryFrom<i64>>::try_from(granted).unwrap_or(0),
            window_left: (ends - now).to_std().unwrap_or_default(),
        })
    }

    /// Trả lại một lần đã đếm (vd lần thử hoá ra không sai): người dùng thật không bị tính oan.
    pub async fn refund(&self, tenant: &TenantContext, key: &str) -> Result<(), Error> {
        let update = Query::update()
            .table(Alias::new(&self.table))
            .value(Alias::new("count"), Expr::col(Alias::new("count")).sub(1))
            .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
            .and_where(Expr::col(Alias::new("key")).eq(sha256(key)))
            .and_where(Expr::col(Alias::new("count")).gt(0))
            .to_owned();
        tenant.database().write().execute(&update).await?;
        Ok(())
    }

    /// Quên mọi lần đã đếm của `key` (vd đăng nhập đúng).
    pub async fn clear(&self, tenant: &TenantContext, key: &str) -> Result<(), Error> {
        let delete = Query::delete()
            .from_table(Alias::new(&self.table))
            .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
            .and_where(Expr::col(Alias::new("key")).eq(sha256(key)))
            .to_owned();
        tenant.database().write().execute(&delete).await?;
        Ok(())
    }

    /// Xoá các dòng (mọi khách) của `database` có cửa sổ đã qua trước `before` và không còn bị
    /// chặn. Module gọi trong job dọn dẹp định kỳ của mình. Trả số dòng đã xoá.
    pub async fn purge(&self, database: &Database, before: DateTime<Utc>) -> Result<u64, Error> {
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
        Ok(database.write().execute(&delete).await?.rows_affected())
    }

    /// Vượt ngưỡng thì chặn tới `now + lock` và đếm lại từ đầu, để hết chặn là có đủ số lần
    /// như mới.
    async fn lock_if_exceeded(
        &self,
        tenant: &TenantContext,
        hashed: &str,
        now: DateTime<Utc>,
    ) -> Result<(), Error> {
        let update = Query::update()
            .table(Alias::new(&self.table))
            .value(Alias::new("locked_until"), Some(now + self.lock))
            .value(Alias::new("count"), 0)
            .value(Alias::new("window_started_at"), now)
            .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
            .and_where(Expr::col(Alias::new("key")).eq(hashed))
            // Lượt vượt mức mới chặn: `limit` lượt đầu vẫn được.
            .and_where(Expr::col(Alias::new("count")).gt(self.limit))
            .to_owned();
        let locked = tenant.database().write().execute(&update).await?.rows_affected();
        if locked > 0 {
            metrics::counter!("kernel_throttle_locks_total", "table" => self.table.clone())
                .increment(1);
        }
        Ok(())
    }
}

/// Kết quả của [`Throttle::take`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Taken {
    /// Số lượt được cấp, từ 0 tới số đã xin.
    pub granted: u32,
    /// Thời gian còn lại của cửa sổ: lượt được cấp chỉ dùng trong khoảng này; cấp 0 thì thử lại sau
    /// chừng này.
    pub window_left: Duration,
}

/// Kết quả của câu "chèn hoặc cộng": số đếm của chính lượt này và hạn khoá của dòng (nếu có).
struct Counted {
    count: i64,
    locked_until: Option<DateTime<Utc>>,
}

/// Số giây (làm tròn lên, ít nhất 1) của `duration`.
fn seconds_up(duration: chrono::Duration) -> u64 {
    let millis = Ord::max(duration.num_milliseconds(), 0);
    Ord::max(<u64 as TryFrom<i64>>::try_from((millis + 999) / 1000).unwrap_or(1), 1)
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

/// Bảng `<module>_throttle` sau khi nâng theo khách (bước 33 A4): khoá chính `(tenant_id, key)`;
/// index dọn dẹp (mọi khách) `…_window_sweep_idx`. Thêm vào migration của module như
/// [`event::tenant_table`](crate::event::tenant_table).
pub fn tenant_table(module: &str) -> crate::tenancy::migration::TenantTable {
    let table = table_name(module);
    crate::tenancy::migration::TenantTable::new(&table)
        .primary_key(["tenant_id", "key"])
        .sweep_index(format!("{table}_window_sweep_idx"), ["window_started_at"])
        .drop_index(format!("{table}_window_idx"))
}
