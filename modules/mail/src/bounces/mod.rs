//! Thư bị dội và danh sách chặn của mọi thư (D-20261125-mail-bounces).
//!
//! - **Nguồn chắc chắn:** máy chủ nhận từ chối hẳn ĐỊA CHỈ người nhận lúc gửi (SMTP 5xx mã mở rộng 5.1.x / 5.2.x — xem
//!   `transport.rs`). Job gửi ghi một lần "dội cứng" cho (khách, địa chỉ) trong CÙNG transaction với event
//!   `mail.delivery_failed`. DSN gửi về sau và webhook của nhà cung cấp chưa nhận (ROADMAP `[ ]`).
//! - **Chặn:** đủ `AUTOXIDE_MAIL_HARD_BOUNCE_LIMIT` lần dội cứng → địa chỉ bị chặn ở khách đó (không xuyên khách).
//!   `Mailer::send` tới địa chỉ đang chặn không xếp hàng thư, trả `Ok` như thường (không để lộ địa chỉ nào bị chặn
//!   qua API quên mật khẩu), và ghi dấu vết trên dòng: số thư bị bỏ, lần cuối, nguồn (phần đầu `dedupe_key` của bên
//!   gửi, vd `auth.password_reset`) — quản trị tra được "vì sao người này không nhận được thư".
//! - **Gỡ chặn:** `POST /api/v1/mail/bounces/{id}/release` (đặt lại số lần tính chặn). Người dùng đổi sang địa chỉ
//!   khác thì thư tới địa chỉ mới đi bình thường (chặn theo địa chỉ).
//! - **Dữ liệu cá nhân:** địa chỉ email là dữ liệu cá nhân. Không gắn với tài khoản (thư gửi theo địa chỉ, event
//!   `auth.user_deleted` chỉ mang ID) nên yêu cầu của chủ dữ liệu đi theo địa chỉ như newsletter: tra bằng
//!   `GET /api/v1/mail/bounces?email=`, xoá bằng `DELETE /api/v1/mail/bounces/{id}`; mọi dòng tự xoá sau
//!   `AUTOXIDE_MAIL_BOUNCE_RETENTION_DAYS` kể từ lần dội / lần bỏ thư cuối.

pub(crate) mod http;
pub(crate) mod migration;

use std::sync::Arc;

use autoxide_kernel::access::{Permission, Principal, Risk};
use autoxide_kernel::database::Database;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::{Actor, Event, EventInfo, Outbox};
use autoxide_kernel::job::{JobContext, JobHandler};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use lettre::message::Mailbox;
use sea_orm::sea_query::{Expr, ExprTrait as _, OnConflict};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter, Set};
use serde::Serialize;
use uuid::Uuid;

pub(crate) mod entity {
    //! Bảng `mail_bounces`.
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "mail_bounces")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub address: String,
        pub hard_count: i32,
        pub strikes: i32,
        pub last_strike_at: Option<Timestamp>,
        pub last_code: String,
        pub first_at: Timestamp,
        pub last_at: Timestamp,
        pub suppressed_at: Option<Timestamp>,
        pub skipped_count: i32,
        pub last_skipped_at: Option<Timestamp>,
        pub last_skipped_source: Option<String>,
        pub released_at: Option<Timestamp>,
        pub released_by: Option<Uuid>,
        pub updated_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}

    impl autoxide_kernel::tenancy::TenantScoped for Entity {
        fn tenant_column() -> Self::Column {
            Column::TenantId
        }
    }
}

use entity::{Column, Entity};

/// Xem thư bị dội và danh sách chặn (có địa chỉ email).
pub(crate) const BOUNCES_READ: Permission = Permission::new(
    "mail.bounces.read",
    "See which mail addresses of this tenant bounced, why, and which are suppressed.",
)
.with_risk(Risk::SensitiveData);

/// Gỡ chặn, xoá dấu vết của một địa chỉ.
pub(crate) const BOUNCES_MANAGE: Permission = Permission::new(
    "mail.bounces.manage",
    "Take a mail address off this tenant's suppression list, or delete its bounce record.",
)
.with_risk(Risk::SensitiveData);

/// Event: một địa chỉ vừa bị chặn. Chỉ mang ID dòng (địa chỉ tra qua API có permission).
pub(crate) const ADDRESS_SUPPRESSED: EventInfo = EventInfo::new(
    "mail.address_suppressed",
    "A mail address bounced too many times and is now suppressed for this tenant; `bounce_id` is \
     the record (`GET /api/v1/mail/bounces/{id}`).",
);

/// Event: quản trị gỡ chặn một địa chỉ.
pub(crate) const SUPPRESSION_RELEASED: EventInfo = EventInfo::new(
    "mail.suppression_released",
    "A suppressed mail address was taken off the suppression list; `bounce_id` is the record.",
);

#[derive(Debug, Serialize)]
struct Suppressed {
    bounce_id: Uuid,
}

impl Event for Suppressed {
    const NAME: &'static str = "mail.address_suppressed";
}

#[derive(Debug, Serialize)]
struct Released {
    bounce_id: Uuid,
}

impl Event for Released {
    const NAME: &'static str = "mail.suppression_released";
}

/// Mã mở rộng (RFC 3463) chắc chắn là lỗi của ĐỊA CHỈ NGƯỜI NHẬN — chỉ chúng được tính là dội cứng
/// (D-20261125-mail-bounces): `5.1.1` không có hộp thư, `5.1.2` tên miền đích không tồn tại / không nhận thư,
/// `5.1.3` địa chỉ sai cú pháp, `5.1.6` hộp thư đã chuyển đi không chuyển tiếp, `5.1.10` tên miền đích khai
/// "null MX" (không nhận thư). Không tính: `5.1.0` (chung chung), `5.1.7` / `5.1.8` (lỗi địa chỉ / hệ thống của
/// BÊN GỬI), `5.2.1` (hộp thư bị khoá — thường tạm thời hay do chính sách), `5.2.2` (hộp thư đầy, dội mềm),
/// `5.2.3` (thư quá cỡ), và câu trả lời không có mã mở rộng.
pub(crate) const HARD_CODES: [&str; 5] = ["5.1.1", "5.1.2", "5.1.3", "5.1.6", "5.1.10"];

/// Hai lần dội cứng của một địa chỉ chỉ cùng tính vào ngưỡng chặn khi cách nhau ít nhất chừng này: một đợt gửi
/// hỏng trong vài phút không đủ chặn.
pub(crate) const STRIKE_GAP_HOURS: i64 = 24;

/// Lần dội với mã mở rộng `enhanced` có được ghi vào sổ dội không.
pub(crate) fn counts_as_hard(enhanced: Option<&str>) -> bool {
    enhanced.is_some_and(|code| HARD_CODES.contains(&code))
}

/// Địa chỉ chuẩn hoá để tra (`Ada <Ada@Example.com>` → `ada@example.com`); `None` khi không phải địa chỉ.
pub(crate) fn address_of(raw: &str) -> Option<String> {
    let mailbox: Mailbox = raw.parse().ok()?;
    Some(autoxide_kernel::email::normalize(mailbox.email.as_ref()))
}

/// Nguồn của thư (phần đầu khoá chống trùng của bên gửi, vd `auth.password_reset:…` → `auth.password_reset`).
fn source_of(dedupe_key: Option<&str>) -> Option<String> {
    let head = dedupe_key?.split(':').next()?.trim();
    let clean = !head.is_empty()
        && head.len() <= 100
        && head.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-');
    clean.then(|| head.to_owned())
}

/// Một địa chỉ trong sổ dội của khách.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Bounce {
    pub row: entity::Model,
}

/// Lọc danh sách.
#[derive(Debug, Clone, Default)]
pub(crate) struct BounceFilter {
    pub email: Option<String>,
    pub suppressed: Option<bool>,
}

/// Số liệu của khách.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BounceStats {
    pub addresses: u64,
    pub suppressed: u64,
    pub bounced_last_30_days: u64,
    pub skipped_last_30_days: u64,
}

pub(crate) struct Bounces {
    /// `AUTOXIDE_MAIL_HARD_BOUNCE_LIMIT`.
    pub hard_limit: u32,
    /// `AUTOXIDE_MAIL_BOUNCE_RETENTION_DAYS`.
    pub retention_days: u32,
    pub outbox: Outbox,
}

/// Bây giờ, cắt tới micro giây để hai database trả cùng giá trị.
fn now() -> Timestamp {
    Timestamp::now().truncate(jiff::Unit::Microsecond)
}

fn not_found(id: Uuid) -> Error {
    Error::not_found("mail.bounce_not_found", "There is no bounce record `{id}`.")
        .with_param("id", id.to_string())
}

impl Bounces {
    /// Ghi một lần dội cứng của `address` (đã chuẩn hoá) ở khách `tenant`, trong `connection` (transaction của
    /// job gửi). Đủ ngưỡng thì chặn và ghi event. Chịu được hai job dội cùng lúc: chỗ chèn dòng không đụng nhau
    /// (`ON CONFLICT DO NOTHING`), số đếm tăng bằng một câu `UPDATE`.
    pub(crate) async fn record_hard<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        address: &str,
        code: &str,
    ) -> Result<bool, Error> {
        self.record_hard_at(connection, tenant, address, code, now()).await
    }

    /// Như [`record_hard`](Self::record_hard), coi `at` là bây giờ (test đặt thời điểm).
    pub(crate) async fn record_hard_at<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        address: &str,
        code: &str,
        at: Timestamp,
    ) -> Result<bool, Error> {
        let code: String = code.chars().take(20).collect();
        let row = entity::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            address: Set(address.to_owned()),
            hard_count: Set(0),
            strikes: Set(0),
            last_strike_at: Set(None),
            last_code: Set(code.clone()),
            first_at: Set(at),
            last_at: Set(at),
            suppressed_at: Set(None),
            skipped_count: Set(0),
            last_skipped_at: Set(None),
            last_skipped_source: Set(None),
            released_at: Set(None),
            released_by: Set(None),
            updated_at: Set(at),
        };
        Entity::insert(row)
            .on_conflict(
                OnConflict::columns([Column::TenantId, Column::Address]).do_nothing().to_owned(),
            )
            .exec_without_returning(connection)
            .await?;
        // Tính vào ngưỡng chỉ khi lần tính trước đã cách ít nhất `STRIKE_GAP_HOURS` (một câu `UPDATE` có điều
        // kiện: hai job cùng lúc không cùng cộng).
        let gap = at - SignedDuration::from_hours(STRIKE_GAP_HOURS);
        Entity::update_many()
            .col_expr(Column::Strikes, Expr::col(Column::Strikes).add(1))
            .col_expr(Column::LastStrikeAt, Expr::value(Some(at)))
            .filter(Column::Address.eq(address))
            .filter(Column::LastStrikeAt.is_null().or(Column::LastStrikeAt.lte(gap)))
            .scoped(tenant)
            .exec(connection)
            .await?;
        Entity::update_many()
            .col_expr(Column::HardCount, Expr::col(Column::HardCount).add(1))
            .col_expr(Column::LastCode, Expr::value(code))
            .col_expr(Column::LastAt, Expr::value(at))
            .col_expr(Column::UpdatedAt, Expr::value(at))
            .filter(Column::Address.eq(address))
            .scoped(tenant)
            .exec(connection)
            .await?;
        let limit = i32::try_from(self.hard_limit).unwrap_or(i32::MAX);
        let suppressed = Entity::update_many()
            .col_expr(Column::SuppressedAt, Expr::value(Some(at)))
            .filter(Column::Address.eq(address))
            .filter(Column::SuppressedAt.is_null())
            .filter(Column::Strikes.gte(limit))
            .scoped(tenant)
            .exec(connection)
            .await?
            .rows_affected
            > 0;
        metrics::counter!("mail_bounces_total", "kind" => "hard").increment(1);
        if suppressed {
            let id = Entity::find()
                .filter(Column::Address.eq(address))
                .scoped(tenant)
                .one(connection)
                .await?
                .map(|row| row.id)
                .ok_or_else(|| Error::internal("the bounce record vanished while being written"))?;
            self.outbox
                .record(connection, tenant, Actor::system(), None, &Suppressed { bounce_id: id })
                .await?;
            metrics::counter!("mail_suppressed_total").increment(1);
            tracing::warn!(tenant_id = %tenant.id(), bounce_id = %id, "mail address suppressed after repeated hard bounces");
        }
        Ok(suppressed)
    }

    /// `address` đang bị chặn ở khách `tenant`: ghi một thư bị bỏ (nguồn `dedupe_key`) và trả `true`.
    pub(crate) async fn skip_if_suppressed(
        &self,
        tenant: &TenantContext,
        address: &str,
        dedupe_key: Option<&str>,
    ) -> Result<bool, Error> {
        // Đọc trước trên kết nối ĐỌC: thư tới địa chỉ không bị chặn (gần như mọi thư) không chiếm kết nối ghi.
        let suppressed = Entity::find()
            .filter(Column::Address.eq(address))
            .filter(Column::SuppressedAt.is_not_null())
            .scoped(tenant)
            .count(tenant.database().read())
            .await?;
        if suppressed == 0 {
            return Ok(false);
        }
        let at = now();
        let skipped = Entity::update_many()
            .col_expr(Column::SkippedCount, Expr::col(Column::SkippedCount).add(1))
            .col_expr(Column::LastSkippedAt, Expr::value(Some(at)))
            .col_expr(Column::LastSkippedSource, Expr::value(source_of(dedupe_key)))
            .col_expr(Column::UpdatedAt, Expr::value(at))
            .filter(Column::Address.eq(address))
            .filter(Column::SuppressedAt.is_not_null())
            .scoped(tenant)
            .exec(tenant.database().write())
            .await?
            .rows_affected
            > 0;
        if skipped {
            metrics::counter!("mail_skipped_suppressed_total").increment(1);
            // Không ghi địa chỉ vào log (dữ liệu cá nhân): tra theo khách và nguồn qua API.
            tracing::warn!(
                tenant_id = %tenant.id(),
                source = source_of(dedupe_key).as_deref().unwrap_or("-"),
                "mail not queued: the recipient address is suppressed after hard bounces"
            );
        }
        Ok(skipped)
    }

    /// Một trang sổ dội của khách, mới nhất trước trừ khi `order=asc`.
    pub(crate) async fn list(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        filter: &BounceFilter,
        page: &PageRequest,
    ) -> Result<Page<Bounce>, Error> {
        principal.require(&BOUNCES_READ)?;
        let mut select = Entity::find().scoped(tenant);
        if let Some(raw) = &filter.email {
            let Some(address) = address_of(raw) else {
                return Err(Error::validation(vec![FieldError::new(
                    "email",
                    "email",
                    "must be a mail address, like `ada@example.com`",
                )]));
            };
            select = select.filter(Column::Address.eq(address));
        }
        match filter.suppressed {
            Some(true) => select = select.filter(Column::SuppressedAt.is_not_null()),
            Some(false) => select = select.filter(Column::SuppressedAt.is_null()),
            None => {}
        }
        let rows =
            fetch_page(select, Column::Id, |row| row.id, page, tenant.database().read()).await?;
        Ok(rows.map(|row| Bounce { row }))
    }

    async fn find(&self, tenant: &TenantContext, id: Uuid) -> Result<entity::Model, Error> {
        Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| not_found(id))
    }

    pub(crate) async fn get(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Bounce, Error> {
        principal.require(&BOUNCES_READ)?;
        Ok(Bounce { row: self.find(tenant, id).await? })
    }

    /// Gỡ chặn: thư mới tới địa chỉ được gửi lại; số lần tính chặn về 0 (tổng số lần dội giữ để tra).
    pub(crate) async fn release(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Bounce, Error> {
        principal.require(&BOUNCES_MANAGE)?;
        let before = self.find(tenant, id).await?;
        if before.suppressed_at.is_none() && before.strikes == 0 {
            return Ok(Bounce { row: before });
        }
        let at = now();
        let transaction = tenant.database().begin_write().await?;
        Entity::update_many()
            .col_expr(Column::SuppressedAt, Expr::value(Option::<Timestamp>::None))
            .col_expr(Column::Strikes, Expr::value(0))
            .col_expr(Column::LastStrikeAt, Expr::value(Option::<Timestamp>::None))
            .col_expr(Column::ReleasedAt, Expr::value(Some(at)))
            .col_expr(Column::ReleasedBy, Expr::value(Some(principal.user_id())))
            .col_expr(Column::UpdatedAt, Expr::value(at))
            .filter(Column::Id.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        if before.suppressed_at.is_some() {
            self.outbox
                .record(
                    &transaction,
                    tenant,
                    Actor::from(principal),
                    Some(format!("mail.bounce:{id}")),
                    &Released { bounce_id: id },
                )
                .await?;
        }
        transaction.commit().await?;
        tracing::info!(tenant_id = %tenant.id(), bounce_id = %id, "mail suppression released");
        Ok(Bounce { row: self.find(tenant, id).await? })
    }

    /// Xoá hẳn dấu vết của một địa chỉ (yêu cầu của chủ dữ liệu). Địa chỉ không còn bị chặn.
    pub(crate) async fn delete(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        principal.require(&BOUNCES_MANAGE)?;
        let deleted = Entity::delete_many()
            .filter(Column::Id.eq(id))
            .scoped(tenant)
            .exec(tenant.database().write())
            .await?;
        if deleted.rows_affected == 0 {
            return Err(not_found(id));
        }
        Ok(())
    }

    /// Số liệu của khách: địa chỉ trong sổ, đang chặn, dội và bị bỏ trong 30 ngày.
    pub(crate) async fn stats(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
    ) -> Result<BounceStats, Error> {
        principal.require(&BOUNCES_READ)?;
        let since = now() - SignedDuration::from_hours(24 * 30);
        let read = tenant.database().read();
        let addresses = Entity::find().scoped(tenant).count(read).await?;
        let suppressed = Entity::find()
            .filter(Column::SuppressedAt.is_not_null())
            .scoped(tenant)
            .count(read)
            .await?;
        let bounced =
            Entity::find().filter(Column::LastAt.gte(since)).scoped(tenant).count(read).await?;
        let skipped = Entity::find()
            .filter(Column::LastSkippedAt.gte(since))
            .scoped(tenant)
            .count(read)
            .await?;
        Ok(BounceStats {
            addresses,
            suppressed,
            bounced_last_30_days: bounced,
            skipped_last_30_days: skipped,
        })
    }

    /// Xoá dòng quá thời hạn giữ trên `database` (mọi khách của database đó), tính từ lần dội và lần bỏ thư cuối.
    pub(crate) async fn cleanup(&self, database: &Database, at: Timestamp) -> Result<u64, Error> {
        let cutoff = at - SignedDuration::from_hours(24 * i64::from(self.retention_days));
        // tenant-scope: job theo lịch dọn cả database theo thời hạn giữ chung, không đọc dữ liệu của khách nào.
        let deleted = Entity::delete_many()
            .filter(Column::LastAt.lt(cutoff))
            .filter(Column::LastSkippedAt.is_null().or(Column::LastSkippedAt.lt(cutoff)))
            .exec(database.write())
            .await?;
        Ok(deleted.rows_affected)
    }
}

/// Loại job dọn hằng ngày.
pub(crate) const CLEANUP: &str = "mail.bounces_cleanup";

/// Job dọn sổ dội theo thời hạn giữ.
pub(crate) struct Cleanup(pub Arc<Bounces>);

#[async_trait::async_trait]
impl JobHandler for Cleanup {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let deleted = self.0.cleanup(&job.database, now()).await?;
        tracing::info!(deleted, "mail bounce records past their retention deleted");
        Ok(())
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn dia_chi_va_nguon() {
        assert_eq!(address_of("Ada <Ada@Example.COM>").as_deref(), Some("ada@example.com"));
        assert_eq!(address_of("not an address"), None);
        assert_eq!(
            source_of(Some("auth.password_reset:0193")).as_deref(),
            Some("auth.password_reset")
        );
        assert_eq!(source_of(Some("a b:c")), None);
        assert_eq!(source_of(None), None);
    }
}
