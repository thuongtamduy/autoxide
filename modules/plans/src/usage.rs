//! Hạn mức (bước 35 phần D): giữ chỗ / xác nhận / trả lại nguyên tử, ghi lượng dùng theo kỳ.
//!
//! Giữ chỗ là một transaction ghi trên mặt điều khiển: thêm dòng `plans_reservation` (`held`, có
//! hạn), bảo đảm có dòng `plans_usage`, rồi cộng `used` bằng `rows::adjust` với trần là hạn mức —
//! điều kiện nằm ở `WHERE` (D-181), không "đọc, kiểm, ghi", nên N request cùng lúc không vượt được
//! hạn mức. `max` đọc từ quyền dùng (có thể từ bộ nhớ đệm); phần nguyên tử ở `adjust`.

use std::collections::BTreeMap;

use autoxide_kernel::entitlement::{
    Limit, LimitKind, limit_exceeded, quota_exhausted, reference_reused,
};
use autoxide_kernel::error::Error;
use autoxide_kernel::rows::{self, Bounds};
use autoxide_kernel::tenancy::TenantId;
use chrono::{DateTime, Datelike, Months, NaiveDate, TimeZone, Utc};
use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set, SqlErr,
};
use uuid::Uuid;

use crate::entity::{reservation, usage, usage_record};
use crate::service::{PlansService, now};

/// `period_start` của hạn mức `count` / `amount` (không có kỳ).
pub(crate) const NO_PERIOD: &str = "0000-00-00";

/// Ngày đầu tháng (UTC) chứa `at`, dạng `YYYY-MM-DD`.
pub(crate) fn month_start(at: DateTime<Utc>) -> String {
    format!("{:04}-{:02}-01", at.year(), at.month())
}

/// Số giây từ `at` tới đầu tháng sau (UTC), ít nhất 1.
pub(crate) fn seconds_to_next_month(at: DateTime<Utc>) -> u64 {
    let next = NaiveDate::from_ymd_opt(at.year(), at.month(), 1)
        .and_then(|first| first.checked_add_months(Months::new(1)))
        .and_then(|first| first.and_hms_opt(0, 0, 0))
        .map(|next| Utc.from_utc_datetime(&next));
    next.map_or(1, |next| u64::try_from((next - at).num_seconds()).unwrap_or(1).max(1))
}

/// Lượng đang dùng của một khách: `limit_key` → `used` (kỳ hiện tại cho hạn mức `metered`).
pub(crate) type Usage = BTreeMap<String, i64>;

impl PlansService {
    /// Xem [`Entitlements::reserve`](autoxide_kernel::entitlement::Entitlements::reserve).
    pub(crate) async fn reserve(
        &self,
        tenant: TenantId,
        limit: &Limit,
        amount: i64,
        reference: &str,
    ) -> Result<(), Error> {
        if amount <= 0 || reference.is_empty() || reference.len() > 200 {
            return Err(Error::internal(format!(
                "reserve `{}` needs an amount above 0 and a reference of 1 to 200 bytes",
                limit.key()
            )));
        }
        let entitlements = self.snapshot(tenant).await?;
        // Khách không giới hạn: không cần sổ (đối soát đếm lại khi khách vào gói).
        if entitlements.unrestricted {
            return Ok(());
        }
        let max = entitlements.limit(limit.key());
        let at = now();
        let id = tenant.as_uuid();
        let transaction = self.databases.control().begin_write().await?;
        let held = reservation::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(id),
            limit_key: Set(limit.key().to_owned()),
            reference: Set(reference.to_owned()),
            amount: Set(amount),
            status: Set("held".to_owned()),
            expires_at: Set(at + self.hold),
            created_at: Set(at),
            updated_at: Set(at),
        }
        .insert(&transaction)
        .await;
        if let Err(error) = held {
            if !matches!(error.sql_err(), Some(SqlErr::UniqueConstraintViolation(_))) {
                return Err(error.into());
            }
            // Gọi lại cùng `reference`: cùng lượng và còn giữ → không giữ thêm.
            drop(transaction);
            let existing = reservation::Entity::find()
                .filter(reservation::Column::TenantId.eq(id))
                .filter(reservation::Column::LimitKey.eq(limit.key()))
                .filter(reservation::Column::Reference.eq(reference))
                .one(self.databases.control().write())
                .await?;
            return match existing {
                Some(row) if row.amount == amount && row.status != "released" => Ok(()),
                Some(row) if row.status == "released" => {
                    // Đã trả lại rồi giữ lại cùng `reference` (vd khôi phục bản ghi): giữ như mới.
                    reservation::Entity::delete_by_id(row.id)
                        .exec(self.databases.control().write())
                        .await?;
                    Box::pin(self.reserve(tenant, limit, amount, reference)).await
                }
                _ => Err(reference_reused(reference, limit.key())),
            };
        }
        ensure_usage(&transaction, id, limit.key(), NO_PERIOD).await?;
        let ceiling = max.unwrap_or(i64::MAX);
        let changed = rows::adjust::<usage::Entity, _>(
            &transaction,
            usage::Column::Used,
            usage_filter(id, limit.key(), NO_PERIOD),
            amount,
            Bounds { floor: 0, ceiling },
        )
        .await?;
        if changed == 0 {
            drop(transaction);
            let used = self.used(tenant, limit.key(), NO_PERIOD).await?;
            return Err(limit_exceeded(limit.key(), used, ceiling));
        }
        transaction.commit().await?;
        Ok(())
    }

    /// Chỗ đã giữ thành vĩnh viễn. Đã xác nhận rồi thì `Ok`; không có chỗ giữ của khách đang bị
    /// giới hạn là lỗi của module gọi (quên `reserve`).
    pub(crate) async fn confirm(
        &self,
        tenant: TenantId,
        limit: &Limit,
        reference: &str,
    ) -> Result<(), Error> {
        let at = now();
        let changed = reservation::Entity::update_many()
            .col_expr(reservation::Column::Status, sea_orm::sea_query::Expr::value("confirmed"))
            .col_expr(reservation::Column::UpdatedAt, sea_orm::sea_query::Expr::value(at))
            .filter(reservation::Column::TenantId.eq(tenant.as_uuid()))
            .filter(reservation::Column::LimitKey.eq(limit.key()))
            .filter(reservation::Column::Reference.eq(reference))
            .filter(reservation::Column::Status.eq("held"))
            .exec(self.databases.control().write())
            .await?
            .rows_affected;
        if changed > 0 {
            return Ok(());
        }
        let existing = reservation::Entity::find()
            .filter(reservation::Column::TenantId.eq(tenant.as_uuid()))
            .filter(reservation::Column::LimitKey.eq(limit.key()))
            .filter(reservation::Column::Reference.eq(reference))
            .one(self.databases.control().write())
            .await?;
        match existing {
            Some(row) if row.status == "confirmed" => Ok(()),
            // Khách không giới hạn không ghi chỗ giữ (xem `reserve`).
            None if self.snapshot(tenant).await?.unrestricted => Ok(()),
            _ => Err(Error::internal(format!(
                "confirm `{}` `{reference}`: there is no hold (it expired, or reserve was not called)",
                limit.key()
            ))),
        }
    }

    /// Trả lại chỗ đã giữ hay đã xác nhận; gọi lại hay `reference` lạ đều `Ok`.
    pub(crate) async fn release(
        &self,
        tenant: TenantId,
        limit: &Limit,
        reference: &str,
    ) -> Result<(), Error> {
        self.release_key(tenant, limit.key(), reference, false).await
    }

    /// Như [`release`](Self::release) theo khoá hạn mức; `only_held`: chỉ trả chỗ còn `held` (job
    /// hết hạn giữ không trả nhầm chỗ vừa được xác nhận).
    async fn release_key(
        &self,
        tenant: TenantId,
        key: &str,
        reference: &str,
        only_held: bool,
    ) -> Result<(), Error> {
        let id = tenant.as_uuid();
        let transaction = self.databases.control().begin_write().await?;
        let Some(row) = rows::lock_one(
            &transaction,
            reservation::Entity::find()
                .filter(reservation::Column::TenantId.eq(id))
                .filter(reservation::Column::LimitKey.eq(key))
                .filter(reservation::Column::Reference.eq(reference)),
        )
        .await?
        else {
            return Ok(());
        };
        if row.status == "released" || (only_held && row.status != "held") {
            return Ok(());
        }
        let amount = row.amount;
        let mut active: reservation::ActiveModel = row.into();
        active.status = Set("released".to_owned());
        active.updated_at = Set(now());
        active.update(&transaction).await?;
        let changed = rows::adjust::<usage::Entity, _>(
            &transaction,
            usage::Column::Used,
            usage_filter(id, key, NO_PERIOD),
            -amount,
            Bounds::NON_NEGATIVE,
        )
        .await?;
        if changed == 0 {
            // Số đang dùng đã lệch (nhỏ hơn chỗ trả): job đối soát đặt lại.
            tracing::warn!(tenant_id = %tenant, limit = key, "usage below a released hold; reconcile will fix it");
            metrics::counter!("plans_usage_drift_total").increment(1);
        }
        transaction.commit().await?;
        Ok(())
    }

    /// Ghi lượng dùng `quantity` của hạn mức `metered` vào kỳ hiện tại; trùng `reference` thì bỏ
    /// qua. Vượt mức của kỳ → `quota_exhausted` (không ghi).
    pub(crate) async fn record_usage(
        &self,
        tenant: TenantId,
        limit: &Limit,
        quantity: i64,
        reference: &str,
    ) -> Result<(), Error> {
        if !matches!(limit.kind(), LimitKind::Metered { .. }) || quantity <= 0 {
            return Err(Error::internal(format!(
                "record_usage `{}` needs a metered limit and a quantity above 0",
                limit.key()
            )));
        }
        let entitlements = self.snapshot(tenant).await?;
        let at = now();
        let period = month_start(at);
        let id = tenant.as_uuid();
        let transaction = self.databases.control().begin_write().await?;
        let inserted = usage_record::Entity::insert(usage_record::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(id),
            limit_key: Set(limit.key().to_owned()),
            period_start: Set(period.clone()),
            quantity: Set(quantity),
            reference: Set(reference.to_owned()),
            recorded_at: Set(at),
        })
        .on_conflict(
            OnConflict::columns([
                usage_record::Column::TenantId,
                usage_record::Column::LimitKey,
                usage_record::Column::Reference,
            ])
            .do_nothing()
            .to_owned(),
        )
        .exec_without_returning(&transaction)
        .await?;
        if inserted == 0 {
            return Ok(());
        }
        ensure_usage(&transaction, id, limit.key(), &period).await?;
        let ceiling = entitlements.limit(limit.key()).unwrap_or(i64::MAX);
        let changed = rows::adjust::<usage::Entity, _>(
            &transaction,
            usage::Column::Used,
            usage_filter(id, limit.key(), &period),
            quantity,
            Bounds { floor: 0, ceiling },
        )
        .await?;
        if changed == 0 {
            drop(transaction);
            return Err(quota_exhausted(limit.key(), seconds_to_next_month(at)));
        }
        transaction.commit().await?;
        Ok(())
    }

    /// Trả các chỗ `held` đã quá hạn (tiến trình giữ chỗ chết, hay module quên xác nhận), tối đa
    /// `batch` chỗ một lượt. Trả số chỗ đã trả.
    pub(crate) async fn expire_holds(&self, at: DateTime<Utc>, batch: u64) -> Result<usize, Error> {
        use sea_orm::{QueryOrder, QuerySelect};
        let due = reservation::Entity::find()
            .filter(reservation::Column::Status.eq("held"))
            .filter(reservation::Column::ExpiresAt.lt(at))
            .order_by_asc(reservation::Column::ExpiresAt)
            .limit(batch)
            .all(self.databases.control().read())
            .await?;
        for row in &due {
            let tenant = TenantId::from_uuid(row.tenant_id);
            self.release_key(tenant, &row.limit_key, &row.reference, true).await?;
        }
        if !due.is_empty() {
            tracing::info!(released = due.len(), "expired limit holds released");
        }
        Ok(due.len())
    }

    /// Đặt lại số đang dùng của mọi hạn mức `count` / `amount` có bộ đếm thật, cho mọi khách có thuê
    /// bao: `used` = số đếm thật + Σ chỗ `held` còn hạn. Sửa lệch do tiến trình chết giữa lúc giữ chỗ
    /// và lúc ghi, hay do khách vào gói khi đã có dữ liệu. Trả số dòng đã sửa.
    pub(crate) async fn reconcile(&self, at: DateTime<Utc>) -> Result<usize, Error> {
        let catalog = self.entitlements.catalog();
        let limits: Vec<(
            &'static str,
            std::sync::Arc<dyn autoxide_kernel::entitlement::UsageCounter>,
        )> = catalog
            .limits()
            .filter(|found| !matches!(found.limit.kind(), LimitKind::Metered { .. }))
            .filter_map(|found| {
                catalog.counter(found.limit.key()).map(|counter| (found.limit.key(), counter))
            })
            .collect();
        if limits.is_empty() {
            return Ok(0);
        }
        let tenants = crate::entity::subscription::Entity::find()
            .filter(crate::entity::subscription::Column::Status.ne("deleted"))
            .all(self.databases.control().read())
            .await?;
        let mut fixed = 0;
        for row in tenants {
            let tenant = TenantId::from_uuid(row.tenant_id);
            // Khách không còn (đã xoá ở sổ khách): bỏ qua.
            let Ok(context) = self.databases.tenant(tenant).await else {
                continue;
            };
            let held: Vec<reservation::Model> = reservation::Entity::find()
                .filter(reservation::Column::TenantId.eq(row.tenant_id))
                .filter(reservation::Column::Status.eq("held"))
                .filter(reservation::Column::ExpiresAt.gt(at))
                .all(self.databases.control().read())
                .await?;
            for (key, counter) in &limits {
                let real = counter.count(&context).await?;
                let pending: i64 =
                    held.iter().filter(|hold| hold.limit_key == *key).map(|hold| hold.amount).sum();
                let target = real.saturating_add(pending);
                let current = self.used(tenant, key, NO_PERIOD).await?;
                if current == target {
                    continue;
                }
                let write = self.databases.control().write();
                ensure_usage(write, row.tenant_id, key, NO_PERIOD).await?;
                usage::Entity::update_many()
                    .col_expr(usage::Column::Used, sea_orm::sea_query::Expr::value(target))
                    .col_expr(usage::Column::UpdatedAt, sea_orm::sea_query::Expr::value(now()))
                    .filter(usage_filter(row.tenant_id, key, NO_PERIOD))
                    .exec(write)
                    .await?;
                metrics::counter!("plans_usage_reconciled_total").increment(1);
                tracing::info!(tenant_id = %tenant, limit = *key, from = current, to = target, "usage reconciled");
                fixed += 1;
            }
        }
        Ok(fixed)
    }

    /// Lượng đang dùng của hạn mức `key` ở kỳ `period`.
    async fn used(&self, tenant: TenantId, key: &str, period: &str) -> Result<i64, Error> {
        Ok(usage::Entity::find()
            .filter(usage_filter(tenant.as_uuid(), key, period))
            .one(self.databases.control().write())
            .await?
            .map_or(0, |row| row.used))
    }

    /// Lượng đang dùng của mọi hạn mức của khách `tenant` (hạn mức `metered`: kỳ chứa `at`).
    pub(crate) async fn usage(&self, tenant: TenantId, at: DateTime<Utc>) -> Result<Usage, Error> {
        let period = month_start(at);
        let rows = usage::Entity::find()
            .filter(usage::Column::TenantId.eq(tenant.as_uuid()))
            .filter(usage::Column::PeriodStart.is_in([NO_PERIOD.to_owned(), period]))
            .all(self.databases.control().read())
            .await?;
        Ok(rows.into_iter().map(|row| (row.limit_key, row.used)).collect())
    }
}

fn usage_filter(tenant: Uuid, key: &str, period: &str) -> sea_orm::Condition {
    sea_orm::Condition::all()
        .add(usage::Column::TenantId.eq(tenant))
        .add(usage::Column::LimitKey.eq(key))
        .add(usage::Column::PeriodStart.eq(period))
}

/// Bảo đảm có dòng `plans_usage` (chèn với 0, đã có thì thôi) — cả hai database có `ON CONFLICT
/// DO NOTHING`.
async fn ensure_usage<C: ConnectionTrait>(
    connection: &C,
    tenant: Uuid,
    key: &str,
    period: &str,
) -> Result<(), Error> {
    usage::Entity::insert(usage::ActiveModel {
        tenant_id: Set(tenant),
        limit_key: Set(key.to_owned()),
        period_start: Set(period.to_owned()),
        used: Set(0),
        updated_at: Set(now()),
    })
    .on_conflict(
        OnConflict::columns([
            usage::Column::TenantId,
            usage::Column::LimitKey,
            usage::Column::PeriodStart,
        ])
        .do_nothing()
        .to_owned(),
    )
    .exec_without_returning(connection)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ky_thang_va_giay_toi_ky_sau() {
        let at = Utc.with_ymd_and_hms(2026, 1, 31, 23, 59, 0).unwrap();
        assert_eq!(month_start(at), "2026-01-01");
        assert_eq!(seconds_to_next_month(at), 60);
        let end = Utc.with_ymd_and_hms(2026, 12, 15, 0, 0, 0).unwrap();
        assert_eq!(month_start(end), "2026-12-01");
        assert_eq!(seconds_to_next_month(end), 17 * 86_400);
    }
}
