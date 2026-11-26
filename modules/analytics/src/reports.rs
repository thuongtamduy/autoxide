//! Báo cáo: CHỈ đọc bảng gộp (`analytics_daily`, `analytics_funnel_daily`) — nhanh trên dữ liệu lớn
//! (D-190), không quét sự kiện thô. Mọi báo cáo là của một website (nút của cây tổ chức; mặc định nút
//! gốc = website của cả khách) trong một khoảng ngày UTC, tối đa 400 ngày một lần. Người gọi cần
//! `analytics.read` tại nút của website.
//!
//! "Khách" trong một khoảng nhiều ngày là tổng khách của từng ngày: phiên không cookie không nối được
//! hai ngày, nên một người ghé hai ngày được đếm hai lần.

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::pagination::{Page, PageRequest};
use autoxide_kernel::tenancy::{OrgId, Site, TenantContext, root_of};
use autoxide_kernel::time::Date;
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Func, Order, Query};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use serde::Serialize;
use utoipa::ToSchema;

use crate::entity::{daily, funnel_daily};
use crate::errors;
use crate::permissions::ANALYTICS_READ;
use crate::service::{AnalyticsService, day_text, now};

/// Số ngày tối đa một lần hỏi.
pub(crate) const RANGE_MAX_DAYS: i64 = 400;
/// Khoảng mặc định: 30 ngày tới hôm nay.
const DEFAULT_DAYS: i64 = 30;
/// Số dòng tối đa của xếp hạng dùng nội bộ (gợi ý của `shared`); danh sách qua API phân trang.
pub(crate) const TOP_MAX: u64 = 100;

/// Website và khoảng ngày đã kiểm.
#[derive(Debug, Clone)]
pub(crate) struct Range {
    pub site: OrgId,
    pub from: Date,
    pub to: Date,
}

impl Range {
    fn first_day(&self) -> String {
        day_text(self.from)
    }

    fn last_day(&self) -> String {
        day_text(self.to)
    }
}

/// One row of a ranking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub(crate) struct Ranked {
    /// The page path, source, campaign, object (`<kind>:<id>`) or event type.
    pub value: String,
    /// Page views (for objects: events about the object).
    pub views: i64,
    /// Sum of the daily unique visitors.
    pub visitors: i64,
    /// Events (for event types and objects).
    pub events: i64,
}

/// One day of the overview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub(crate) struct DayCount {
    #[schema(value_type = String, format = Date)]
    pub day: Date,
    pub views: i64,
    pub visitors: i64,
}

/// Visits of a website over a range of days.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct Overview {
    /// The website (organization unit).
    pub site: uuid::Uuid,
    #[schema(value_type = String, format = Date)]
    pub from: Date,
    #[schema(value_type = String, format = Date)]
    pub to: Date,
    /// Sum of the daily unique visitors (no cookies: one person on two days counts twice).
    pub visitors: i64,
    /// Page views.
    pub views: i64,
    /// All events.
    pub events: i64,
    /// Visits that saw exactly one page.
    pub bounces: i64,
    /// `bounces / visitors`, 0 to 1 (0 when there are no visitors).
    pub bounce_rate: f64,
    /// The sources bringing the most page views from outside the website.
    pub top_sources: Vec<Ranked>,
    /// One entry per day that had visits.
    pub days: Vec<DayCount>,
}

/// One step of a funnel.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct FunnelStep {
    /// Position, from 0.
    pub step: i32,
    /// The event type of the step.
    pub key: String,
    /// Sessions with the event, summed over the days.
    pub sessions: i64,
    /// `sessions` of this step divided by the first step's, 0 to 1 and more (steps are counted
    /// independently, see the funnel endpoint).
    pub from_start: f64,
    /// `sessions` of this step divided by the previous step's.
    pub from_previous: f64,
}

/// A funnel over a range of days.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub(crate) struct FunnelReport {
    pub key: String,
    pub description: String,
    #[schema(value_type = String, format = Date)]
    pub from: Date,
    #[schema(value_type = String, format = Date)]
    pub to: Date,
    pub steps: Vec<FunnelStep>,
}

/// Chiều xếp hạng được hỏi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dimension {
    Page,
    Source,
    Campaign,
    Object,
    Type,
}

impl Dimension {
    fn as_str(self) -> &'static str {
        match self {
            Dimension::Page => "page",
            Dimension::Source => "source",
            Dimension::Campaign => "campaign",
            Dimension::Object => "object",
            Dimension::Type => "type",
        }
    }
}

fn ratio(numerator: i64, denominator: i64) -> f64 {
    if denominator <= 0 {
        return 0.0;
    }
    // Số đếm nhỏ hơn 2^53: đổi sang f64 không mất chính xác đáng kể; làm tròn 4 chữ số.
    let value = numerator as f64 / denominator as f64;
    (value * 10_000.0).round() / 10_000.0
}

impl AnalyticsService {
    /// Kiểm website và khoảng ngày rồi kiểm `analytics.read` tại nút của website.
    pub(crate) async fn range(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        site: Option<uuid::Uuid>,
        from: Option<Date>,
        to: Option<Date>,
    ) -> Result<Range, Error> {
        let to = to.unwrap_or_else(|| now().date_utc());
        let from = from.unwrap_or(to.checked_sub_days(DEFAULT_DAYS - 1).unwrap_or(to));
        if from > to {
            return Err(errors::range_reversed());
        }
        if to.days_since(from) + 1 > RANGE_MAX_DAYS {
            return Err(errors::range_too_long(RANGE_MAX_DAYS));
        }
        let site = match site {
            None => root_of(tenant.id()),
            Some(id) => {
                let org = OrgId::from_uuid(id);
                Site::of(tenant, org).await.map_err(|_| errors::site_unknown(id))?.org()
            }
        };
        principal
            .require_at(tenant.database().read(), &ANALYTICS_READ, OrgTarget::new(site, None))
            .await?;
        Ok(Range { site, from, to })
    }

    /// Tổng quan của website trong khoảng ngày.
    pub(crate) async fn overview(
        &self,
        tenant: &TenantContext,
        range: &Range,
    ) -> Result<Overview, Error> {
        let rows = daily::Entity::find()
            .filter(daily::Column::TenantId.eq(tenant.id().as_uuid()))
            .filter(daily::Column::SiteOrgId.eq(range.site.as_uuid()))
            .filter(daily::Column::Dimension.eq("total"))
            .filter(daily::Column::Day.between(range.first_day(), range.last_day()))
            .order_by_asc(daily::Column::Day)
            .all(tenant.database().read())
            .await?;
        let sum = |pick: fn(&daily::Model) -> i64| rows.iter().map(pick).sum::<i64>();
        let (visitors, bounces) = (sum(|row| row.visitors), sum(|row| row.bounces));
        let days = rows
            .iter()
            .filter_map(|row| {
                Date::parse_from_str(&row.day, "%Y-%m-%d").ok().map(|day| DayCount {
                    day,
                    views: row.views,
                    visitors: row.visitors,
                })
            })
            .collect();
        Ok(Overview {
            site: range.site.as_uuid(),
            from: range.from,
            to: range.to,
            visitors,
            views: sum(|row| row.views),
            events: sum(|row| row.events),
            bounces,
            bounce_rate: ratio(bounces, visitors),
            top_sources: self.top(tenant, range, Dimension::Source, None, 5).await?,
            days,
        })
    }

    /// Một trang của bảng xếp hạng theo một chiều (REST và tool): mọi giá trị trong khoảng ngày, nhiều nhất
    /// trước (`order=asc`, mặc định), cùng số thì theo giá trị; `order=desc` đảo lại. Phân trang bằng keyset
    /// ngay trong SQL trên tổng của nhóm (`HAVING`, D-20261126-list-pages-cluster-3): database chỉ trả
    /// `limit + 1` nhóm, không kéo mọi nhóm của khoảng ngày về bộ nhớ. Số được cộng thêm mỗi giờ: một giá
    /// trị đổi hạng trong lúc duyệt có thể lặp hay sót — chấp nhận, như mọi danh sách sắp theo giá trị đổi được.
    pub(crate) async fn ranking_page(
        &self,
        tenant: &TenantContext,
        range: &Range,
        dimension: Dimension,
        prefix: Option<&str>,
        page: &PageRequest,
    ) -> Result<Page<Ranked>, Error> {
        let after =
            page.text_key_after(RANK_CURSOR)?.map(|key| parse_rank_key(&key)).transpose()?;
        let most_first = page.order() == autoxide_kernel::pagination::Order::Asc;
        let rank = rank_expr(dimension);
        let mut select = self.ranking_select(tenant, range, dimension, prefix);
        if let Some((count, value)) = after {
            // Nhiều trước: (tổng < n) hoặc (tổng = n và giá trị > v); đảo chiều thì ngược lại.
            let (beyond, tie) = if most_first {
                (rank.clone().lt(count), Expr::col(daily::Column::Value).gt(value))
            } else {
                (rank.clone().gt(count), Expr::col(daily::Column::Value).lt(value))
            };
            select.and_having(beyond.or(rank.clone().eq(count).and(tie)));
        }
        let (rank_order, value_order) =
            if most_first { (Order::Desc, Order::Asc) } else { (Order::Asc, Order::Desc) };
        select
            .order_by_expr(rank, rank_order)
            .order_by(daily::Column::Value, value_order)
            .limit(u64::from(page.limit()) + 1);
        let mut rows = read_ranked(tenant, &select).await?;
        let has_more = rows.len() > page.limit() as usize;
        rows.truncate(page.limit() as usize);
        let by_events = matches!(dimension, Dimension::Type);
        let next = has_more.then(|| rows.last()).flatten().map(|row| {
            let count = if by_events { row.events } else { row.views };
            autoxide_kernel::pagination::text_key_cursor(
                RANK_CURSOR,
                &format!("{count}\u{1f}{}", row.value),
            )
        });
        let total = if page.wants_total() {
            // Số nhóm (giá trị) của khoảng ngày: đếm trên subquery `GROUP BY`, chạy được trên cả hai database.
            let groups = self.ranking_select(tenant, range, dimension, prefix);
            let count = Query::select()
                .expr(Expr::cust("COUNT(*)"))
                .from_subquery(groups, Alias::new("groups"))
                .to_owned();
            let total: i64 = match tenant.database().read().query_one(&count).await? {
                Some(row) => row.try_get_by_index(0)?,
                None => 0,
            };
            Some(u64::try_from(total).unwrap_or_default())
        } else {
            None
        };
        Ok(Page {
            data: rows,
            pagination: autoxide_kernel::pagination::Pagination::new(page.limit(), next)
                .with_total(total),
        })
    }

    /// Xếp hạng theo một chiều: cộng các ngày, nhiều nhất trước, tối đa `limit` dòng (việc nội bộ: gợi ý
    /// của `shared`). `prefix`: chỉ giá trị bắt đầu bằng chuỗi này (vd `catalog.product:`).
    pub(crate) async fn top(
        &self,
        tenant: &TenantContext,
        range: &Range,
        dimension: Dimension,
        prefix: Option<&str>,
        limit: u64,
    ) -> Result<Vec<Ranked>, Error> {
        self.ranked(tenant, range, dimension, prefix, limit.clamp(1, TOP_MAX)).await
    }

    /// Các nhóm của chiều `dimension` trong khoảng ngày, nhiều nhất trước, tối đa `limit` (`ORDER BY` + `LIMIT`
    /// trong SQL).
    async fn ranked(
        &self,
        tenant: &TenantContext,
        range: &Range,
        dimension: Dimension,
        prefix: Option<&str>,
        limit: u64,
    ) -> Result<Vec<Ranked>, Error> {
        let mut select = self.ranking_select(tenant, range, dimension, prefix);
        select
            .order_by_expr(rank_expr(dimension), Order::Desc)
            .order_by(daily::Column::Value, Order::Asc)
            .limit(limit);
        read_ranked(tenant, &select).await
    }

    /// `SELECT value, SUM(views), SUM(visitors), SUM(events) … GROUP BY value` của chiều trong khoảng ngày (chưa
    /// sắp, chưa giới hạn). `prefix`: chỉ giá trị bắt đầu bằng chuỗi này.
    fn ranking_select(
        &self,
        tenant: &TenantContext,
        range: &Range,
        dimension: Dimension,
        prefix: Option<&str>,
    ) -> sea_orm::sea_query::SelectStatement {
        let mut select = Query::select();
        select
            .from(daily::Entity)
            .column(daily::Column::Value)
            .expr_as(sum_bigint(daily::Column::Views), Alias::new("views"))
            .expr_as(sum_bigint(daily::Column::Visitors), Alias::new("visitors"))
            .expr_as(sum_bigint(daily::Column::Events), Alias::new("events"))
            .and_where(Expr::col(daily::Column::TenantId).eq(tenant.id().as_uuid()))
            .and_where(Expr::col(daily::Column::SiteOrgId).eq(range.site.as_uuid()))
            .and_where(Expr::col(daily::Column::Dimension).eq(dimension.as_str()))
            .and_where(Expr::col(daily::Column::Day).between(range.first_day(), range.last_day()))
            .group_by_col(daily::Column::Value);
        if let Some(prefix) = prefix {
            // `prefix` là loại đối tượng đã kiểm (chữ thường, số, `_`, `.`): không có `%`.
            select.and_where(Expr::col(daily::Column::Value).like(format!("{prefix}%")));
        }
        select
    }

    /// Phễu `key` của website trong khoảng ngày.
    pub(crate) async fn funnel(
        &self,
        tenant: &TenantContext,
        range: &Range,
        key: &str,
    ) -> Result<FunnelReport, Error> {
        let funnel = self.catalog.funnel(key).ok_or_else(|| errors::funnel_unknown(key))?;
        let rows: Vec<(i32, i64)> = funnel_daily::Entity::find()
            .select_only()
            .column(funnel_daily::Column::Step)
            .column_as(sum_bigint(funnel_daily::Column::Sessions), "sessions")
            .filter(funnel_daily::Column::TenantId.eq(tenant.id().as_uuid()))
            .filter(funnel_daily::Column::SiteOrgId.eq(range.site.as_uuid()))
            .filter(funnel_daily::Column::Funnel.eq(key))
            .filter(funnel_daily::Column::Day.between(range.first_day(), range.last_day()))
            .group_by(funnel_daily::Column::Step)
            .into_tuple::<(i32, Option<i64>)>()
            .all(tenant.database().read())
            .await?
            .into_iter()
            .map(|(step, sessions)| (step, sessions.unwrap_or(0)))
            .collect();
        let mut steps: Vec<FunnelStep> = Vec::new();
        for (index, step) in funnel.steps.iter().enumerate() {
            let position = i32::try_from(index).unwrap_or(i32::MAX);
            let sessions =
                rows.iter().find(|(step, _)| *step == position).map_or(0, |(_, count)| *count);
            let first = steps.first().map_or(sessions, |first| first.sessions);
            let previous = steps.last().map_or(sessions, |previous| previous.sessions);
            steps.push(FunnelStep {
                step: position,
                key: (*step).to_owned(),
                sessions,
                from_start: ratio(sessions, first),
                from_previous: ratio(sessions, previous),
            });
        }
        Ok(FunnelReport {
            key: funnel.key.to_owned(),
            description: funnel.description.to_owned(),
            from: range.from,
            to: range.to,
            steps,
        })
    }
}

/// `CAST(SUM(cột) AS BIGINT)`: Postgres trả `numeric` cho tổng của `bigint`, ép về số nguyên để hai
/// database trả cùng kiểu.
/// Tên thứ tự trong con trỏ của bảng xếp hạng (`text_key_cursor`): con trỏ của danh sách khác bị từ chối.
const RANK_CURSOR: &str = "rank";

/// Tổng xếp hạng của chiều: số sự kiện cho `type`, số lượt xem cho chiều khác.
fn rank_expr(dimension: Dimension) -> Expr {
    if matches!(dimension, Dimension::Type) {
        sum_bigint(daily::Column::Events)
    } else {
        sum_bigint(daily::Column::Views)
    }
}

/// Khoá con trỏ `<tổng>\u{1f}<giá trị>`; hỏng thì 422 ở `cursor`.
fn parse_rank_key(key: &str) -> Result<(i64, String), Error> {
    key.split_once('\u{1f}')
        .and_then(|(count, value)| Some((count.parse::<i64>().ok()?, value.to_owned())))
        .ok_or_else(|| {
            Error::validation(vec![autoxide_kernel::error::FieldError::new(
                "cursor",
                "invalid",
                "must be the `next_cursor` value of a previous page of the same list and sort, unchanged",
            )])
        })
}

/// Đọc các dòng của câu xếp hạng.
async fn read_ranked(
    tenant: &TenantContext,
    select: &sea_orm::sea_query::SelectStatement,
) -> Result<Vec<Ranked>, Error> {
    let rows = tenant.database().read().query_all(select).await?;
    let mut ranked = Vec::with_capacity(rows.len());
    for row in rows {
        ranked.push(Ranked {
            value: row.try_get("", "value")?,
            views: row.try_get::<Option<i64>>("", "views")?.unwrap_or(0),
            visitors: row.try_get::<Option<i64>>("", "visitors")?.unwrap_or(0),
            events: row.try_get::<Option<i64>>("", "events")?.unwrap_or(0),
        });
    }
    Ok(ranked)
}

pub(crate) fn sum_bigint(column: impl sea_orm::sea_query::IntoColumnRef) -> Expr {
    Func::cast_as(Func::sum(Expr::col(column)), Alias::new("BIGINT")).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ti_le() {
        assert!((ratio(1, 3) - 0.3333).abs() < f64::EPSILON);
        assert!(ratio(5, 0).abs() < f64::EPSILON);
    }
}
