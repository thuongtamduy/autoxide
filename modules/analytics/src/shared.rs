//! Phần module `analytics` cho module khác và cho trang tổng quan: cài đặt trait [`Analytics`] của
//! `api` ("thịnh hành", "vừa xem") và số liệu tóm tắt `analytics.visitors` (bước 51 P4).

use autoxide_kernel::access::{OrgTarget, Permission, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::summary::{MetricInfo, MetricValue, SummaryMetric};
use autoxide_kernel::tenancy::{OrgId, TenantContext, root_of};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use uuid::Uuid;

use crate::api::{Analytics, ObjectViews};
use crate::entity::{daily, event};
use crate::errors;
use crate::permissions::ANALYTICS_READ;
use crate::reports::{Dimension, RANGE_MAX_DAYS, Range, TOP_MAX, sum_bigint};
use crate::service::{AnalyticsService, day_text, now};

/// `<module>.<loại>`, chữ thường, số, `_`.
pub(crate) fn valid_kind(kind: &str) -> bool {
    let identifier = |text: &str| {
        !text.is_empty()
            && text.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    };
    kind.split_once('.').is_some_and(|(module, rest)| identifier(module) && identifier(rest))
}

#[async_trait::async_trait]
impl Analytics for AnalyticsService {
    async fn top_objects(
        &self,
        tenant: &TenantContext,
        site: Option<OrgId>,
        kind: &str,
        days: u32,
        limit: u32,
    ) -> Result<Vec<ObjectViews>, Error> {
        if !valid_kind(kind) {
            return Err(errors::kind_invalid());
        }
        let days = i64::from(days).clamp(1, RANGE_MAX_DAYS);
        let to = now().date_utc();
        let range = Range {
            site: site.unwrap_or_else(|| root_of(tenant.id())),
            from: to.checked_sub_days(days - 1).unwrap_or(to),
            to,
        };
        let prefix = format!("{kind}:");
        let ranked = self
            .top(tenant, &range, Dimension::Object, Some(&prefix), u64::from(limit).min(TOP_MAX))
            .await?;
        Ok(ranked
            .into_iter()
            .map(|row| ObjectViews {
                id: row.value.strip_prefix(&prefix).unwrap_or_default().to_owned(),
                object: row.value,
                views: row.views,
                visitors: row.visitors,
            })
            .collect())
    }

    async fn recently_viewed(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        kind: &str,
        limit: u32,
    ) -> Result<Vec<String>, Error> {
        if !valid_kind(kind) {
            return Err(errors::kind_invalid());
        }
        // Rút đồng ý thì ngừng ngay, kể cả khi bên nhận event chưa kịp gỡ `user_id`.
        if !self.personal_allowed(tenant, user_id).await? {
            return Ok(Vec::new());
        }
        let limit = usize::try_from(limit.clamp(1, 100)).unwrap_or(100);
        let rows: Vec<Option<String>> = event::Entity::find()
            .select_only()
            .column(event::Column::Object)
            .filter(event::Column::TenantId.eq(tenant.id().as_uuid()))
            .filter(event::Column::UserId.eq(user_id))
            .filter(event::Column::Object.like(format!("{kind}:%")))
            .order_by_desc(event::Column::OccurredAt)
            .order_by_desc(event::Column::Id)
            // Đủ để lọc trùng mà không đọc hết.
            .limit(u64::try_from(limit * 10).unwrap_or(1000))
            .into_tuple()
            .all(tenant.database().read())
            .await?;
        let mut seen = Vec::new();
        for object in rows.into_iter().flatten() {
            if !seen.contains(&object) {
                seen.push(object);
            }
            if seen.len() == limit {
                break;
            }
        }
        Ok(seen)
    }
}

/// Số liệu tóm tắt: khách 7 ngày qua trên website của nút được chọn.
pub(crate) struct Visitors;

#[async_trait::async_trait]
impl SummaryMetric for Visitors {
    fn info(&self) -> MetricInfo {
        MetricInfo::new(
            "analytics.visitors",
            "Visitors, last 7 days",
            "Sum of the daily unique visitors of the website of the chosen unit over the last 7 \
             days (today included, UTC), from the hourly totals; no cookies, so one person on two \
             days counts twice.",
            "marketing",
        )
    }

    fn permission(&self) -> Permission {
        ANALYTICS_READ
    }

    async fn value(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        target: OrgTarget,
    ) -> Result<MetricValue, Error> {
        principal.require_at(tenant.database().read(), &ANALYTICS_READ, target).await?;
        let to = now().date_utc();
        let from = to.checked_sub_days(6).unwrap_or(to);
        let visitors: Option<i64> = daily::Entity::find()
            .select_only()
            .column_as(sum_bigint(daily::Column::Visitors), "visitors")
            .filter(daily::Column::TenantId.eq(tenant.id().as_uuid()))
            .filter(daily::Column::SiteOrgId.eq(target.org.as_uuid()))
            .filter(daily::Column::Dimension.eq("total"))
            .filter(daily::Column::Day.between(day_text(from), day_text(to)))
            .into_tuple::<Option<i64>>()
            .one(tenant.database().read())
            .await?
            .flatten();
        Ok(MetricValue::Count(u64::try_from(visitors.unwrap_or(0)).unwrap_or(0)))
    }
}
