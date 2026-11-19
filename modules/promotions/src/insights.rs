//! Chạy thử, xung đột, lịch, số liệu của chương trình (D-222). Chỉ đọc, không ghi gì.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use chrono::{DateTime, NaiveDate, Utc};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::api::{Cart, Evaluation};
use crate::compiled::program_of;
use crate::entity::stats;
use crate::evaluate::Options;
use crate::model::{Exclusivity, Kind, Status};
use crate::service::PromotionsService;

/// Số giỏ tối đa mỗi lần chạy thử.
pub(crate) const MAX_SIMULATE_CARTS: usize = 20;
/// Khoảng ngày dài nhất của số liệu và lịch.
pub(crate) const MAX_RANGE_DAYS: i64 = 366;

/// One cart computed with and without the promotion.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = PromotionsSimulation)]
pub(crate) struct Simulation {
    /// With the promotion (counted even while it is a draft or paused).
    pub with: Evaluation,
    /// Without it.
    pub without: Evaluation,
    /// What the promotion adds to the discount of this cart, in minor units (can be negative
    /// when it pushes out a better promotion).
    pub difference_minor: i64,
}

/// Another promotion that may clash with this one.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = PromotionsConflict)]
pub(crate) struct Conflict {
    pub id: Uuid,
    #[schema(inline)]
    pub kind: Kind,
    #[schema(inline)]
    pub status: Status,
    #[schema(inline)]
    pub exclusivity: Exclusivity,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
}

/// Counts of one day (Vietnam date).
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = PromotionsDailyStats)]
pub(crate) struct DailyStats {
    pub day: NaiveDate,
    /// Confirmed uses.
    pub redemptions: i64,
    /// Discount given, in minor units.
    pub discount_minor: i64,
    /// Value of the orders that used it, in minor units.
    pub order_value_minor: i64,
    pub platform_funded_minor: i64,
    pub seller_funded_minor: i64,
}

/// Lỗi khoảng ngày: `from` sau `to` hay dài quá.
pub(crate) fn check_range(from: NaiveDate, to: NaiveDate) -> Result<(), Error> {
    if from > to {
        return Err(Error::validation(vec![FieldError::new(
            "from",
            "range",
            "must not be after `to`",
        )]));
    }
    if (to - from).num_days() >= MAX_RANGE_DAYS {
        return Err(Error::validation(vec![
            FieldError::new("to", "range", "the range must be at most {max} days")
                .with_param("max", MAX_RANGE_DAYS),
        ]));
    }
    Ok(())
}

impl PromotionsService {
    /// Tính từng giỏ có và không có chương trình `id` (kể cả bản nháp). Không ghi gì.
    pub(crate) async fn simulate(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        carts: &[Cart],
    ) -> Result<Vec<Simulation>, Error> {
        if carts.is_empty() || carts.len() > MAX_SIMULATE_CARTS {
            return Err(Error::validation(vec![
                FieldError::new("carts", "length", "send 1 to {max} carts")
                    .with_param("max", MAX_SIMULATE_CARTS),
            ]));
        }
        let view = self.promotion(tenant, principal, id).await?;
        let program = Arc::new(program_of(&view));
        let mut out = Vec::with_capacity(carts.len());
        for cart in carts {
            let with = Options { include: Some(program.clone()), ..Options::default() };
            let without = Options { exclude: Some(id), ..Options::default() };
            let with = self.evaluate_cart(tenant, cart, &with).await?.evaluation;
            let without = self.evaluate_cart(tenant, cart, &without).await?.evaluation;
            let difference_minor = with.total_discount.minor() - without.total_discount.minor();
            out.push(Simulation { with, without, difference_minor });
        }
        Ok(out)
    }

    /// Chương trình khác cùng nút, trùng thời gian và mục tiêu, cùng lớp, mà một bên độc quyền.
    pub(crate) async fn conflicts(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Vec<Conflict>, Error> {
        let view = self.promotion(tenant, principal, id).await?;
        let program = program_of(&view);
        Ok(self
            .overlapping_programs(tenant, &program, true)
            .await?
            .into_iter()
            .filter(|other| crate::compliance::clashes(&program, other))
            .map(|other| Conflict {
                id: other.id,
                kind: other.kind,
                status: other.status,
                exclusivity: other.exclusivity,
                starts_at: other.starts_at,
                ends_at: other.ends_at,
            })
            .collect())
    }

    /// Số liệu từng ngày của chương trình `id` trong [from, to] (ngày Việt Nam), ngày cũ trước;
    /// ngày không có lượt dùng thì vắng.
    pub(crate) async fn stats(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<DailyStats>, Error> {
        check_range(from, to)?;
        let view = self.promotion(tenant, principal, id).await?;
        // `day` lưu dạng chữ `YYYY-MM-DD`: so sánh chữ đúng thứ tự ngày trên cả hai database.
        let rows = stats::Entity::find()
            .filter(stats::Column::PromotionId.eq(view.row.id))
            .filter(stats::Column::Day.gte(from.to_string()))
            .filter(stats::Column::Day.lte(to.to_string()))
            .scoped(tenant)
            .order_by_asc(stats::Column::Day)
            .all(tenant.database().read())
            .await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                Some(DailyStats {
                    day: row.day.parse().ok()?,
                    redemptions: row.redemptions,
                    discount_minor: row.discount_minor,
                    order_value_minor: row.order_value_minor,
                    platform_funded_minor: row.platform_funded_minor,
                    seller_funded_minor: row.seller_funded_minor,
                })
            })
            .collect())
    }
}
