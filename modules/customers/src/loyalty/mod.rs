//! Điểm thưởng và hạng thành viên (bước 47 mục 3, D-226, D-20261120-step47-loyalty).
//!
//! - Quy tắc tích / đổi và hạng là dữ liệu có ngày hiệu lực, chỉ thêm (`admin.rs`).
//! - Mỗi lần tích là một **lô** có hạn riêng; đổi điểm tiêu lô hết hạn sớm nhất trước (`points.rs`).
//! - Số dư (`customers_points_balances`) đổi bằng `rows::adjust` với `Bounds::NON_NEGATIVE`: hai
//!   lần đổi cùng lúc không bao giờ làm âm.
//! - Điểm là một cách trả tiền của `orders` (`Tender` loại `points`, `tender.rs`).
//! - Điểm chưa dùng là nghĩa vụ trong sổ cái, ghi theo giá trị điểm LÚC TÍCH lưu trên lô
//!   (`ledger.rs`).

pub(crate) mod admin;
pub(crate) mod ledger;
pub(crate) mod points;
pub(crate) mod tender;
pub(crate) mod tiers;
pub(crate) mod vouchers;

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use chrono::{DateTime, Duration, FixedOffset, Months, NaiveTime, Offset, TimeZone, Utc};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder};

use crate::entity::{loyalty_rule, tier};

/// Múi giờ tính hạn điểm và ngày sinh: giờ Việt Nam (UTC+7). Lưu UTC.
pub(crate) fn local_offset() -> FixedOffset {
    // 7 giờ luôn nằm trong khoảng hợp lệ của `FixedOffset`; nhánh `UTC` không bao giờ chạy.
    FixedOffset::east_opt(7 * 3600).unwrap_or_else(|| FixedOffset::east_opt(0).unwrap_or(Utc.fix()))
}

/// Cuối ngày (23:59:59 giờ Việt Nam) của `months` tháng sau `from`.
pub(crate) fn expiry_after(from: DateTime<Utc>, months: i32) -> DateTime<Utc> {
    let local = from.with_timezone(&local_offset());
    let later = local
        .checked_add_months(Months::new(u32::try_from(months.max(1)).unwrap_or(12)))
        .unwrap_or(local + Duration::days(365));
    let end = later.date_naive().and_time(NaiveTime::from_hms_opt(23, 59, 59).unwrap_or_default());
    local_offset()
        .from_local_datetime(&end)
        .single()
        .map_or(later.with_timezone(&Utc), |at| at.with_timezone(&Utc))
}

/// Quy tắc đang hiệu lực lúc `at` (dòng có `effective_from` muộn nhất không sau `at`).
pub(crate) async fn current_rule<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    at: DateTime<Utc>,
) -> Result<Option<loyalty_rule::Model>, Error> {
    Ok(loyalty_rule::Entity::find()
        .filter(loyalty_rule::Column::EffectiveFrom.lte(at))
        .scoped(tenant)
        .order_by_desc(loyalty_rule::Column::EffectiveFrom)
        .order_by_desc(loyalty_rule::Column::Id)
        .one(connection)
        .await?)
}

/// Bản đang hiệu lực của mỗi hạng lúc `at`, hạng cao trước.
pub(crate) async fn current_tiers<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    at: DateTime<Utc>,
) -> Result<Vec<tier::Model>, Error> {
    let rows = tier::Entity::find()
        .filter(tier::Column::EffectiveFrom.lte(at))
        .scoped(tenant)
        .order_by_desc(tier::Column::EffectiveFrom)
        .all(connection)
        .await?;
    let mut seen = std::collections::BTreeSet::new();
    let mut current: Vec<tier::Model> =
        rows.into_iter().filter(|row| seen.insert(row.code.clone())).collect();
    current.sort_by(|a, b| b.rank.cmp(&a.rank).then(b.threshold_minor.cmp(&a.threshold_minor)));
    Ok(current)
}

/// `a × b / c` bằng `i128`, làm tròn xuống, không tràn.
pub(crate) fn mul_div(a: i64, b: i64, c: i64) -> i64 {
    if c == 0 {
        return 0;
    }
    i64::try_from(i128::from(a) * i128::from(b) / i128::from(c)).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn han_diem_la_cuoi_ngay_gio_viet_nam() {
        let from = "2026-01-31T20:00:00Z".parse::<DateTime<Utc>>().unwrap();
        // 03:00 ngày 01/02 giờ Việt Nam + 12 tháng → 23:59:59 ngày 01/02/2027 giờ Việt Nam.
        assert_eq!(expiry_after(from, 12).to_rfc3339(), "2027-02-01T16:59:59+00:00");
        assert_eq!(mul_div(i64::MAX, 3, 4), i64::MAX / 4 * 3 + 2);
        assert_eq!(mul_div(10, 12_000, 10_000), 12);
    }
}
