//! Job gộp `analytics.rollup` (mỗi giờ): tính lại số gộp theo ngày từ sự kiện thô cho từng khách
//! có dữ liệu trên database của job. Ngày được tính lại: hôm nay, hai ngày trước (sự kiện phía máy
//! chủ có thể tới muộn), và mọi ngày còn sự kiện thô mà chưa từng gộp (máy chủ tắt lâu). Tính lại
//! = xoá số gộp của ngày rồi ghi mới trong một transaction: chạy lại bao nhiêu lần cũng ra cùng kết
//! quả.
//!
//! Chiều (`analytics_daily.dimension`):
//! - `total` (`value` rỗng): lượt xem trang, khách (phiên trình duyệt khác nhau), mọi sự kiện, số
//!   phiên thoát (chỉ xem đúng một trang);
//! - `page` (đường dẫn), `source` và `campaign` (chỉ lượt xem đến từ ngoài website): lượt xem, khách;
//! - `object` (`<loại>:<id>`), `type` (loại sự kiện): số sự kiện, khách.
//!
//! Phễu (`analytics_funnel_daily`): mỗi bước đếm số phiên có sự kiện của bước đó trong ngày. Phiên
//! không cookie không nối được với sự kiện phía máy chủ (đơn đặt), nên các bước đếm độc lập, không
//! đòi cùng một phiên đi qua mọi bước (D-20261118-step57-analytics).

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::job::{JobContext, JobHandler};
use autoxide_kernel::tenancy::{TenantContext, TenantId};
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Func, Query, SimpleExpr};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QuerySelect, Set};
use uuid::Uuid;

use crate::entity::{daily, event, funnel_daily, rollup};
use crate::service::{AnalyticsService, PAGE_VIEW, day_of, now};

pub(crate) const ROLLUP: &str = "analytics.rollup";

/// Số ngày gần đây luôn được gộp lại (hôm nay và hai ngày trước).
const RECENT_DAYS: i64 = 2;
/// Số dòng mỗi câu `INSERT`.
const CHUNK: usize = 500;

pub(crate) struct Rollup(pub Arc<AnalyticsService>);

#[async_trait::async_trait]
impl JobHandler for Rollup {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.0.rollup_database(&job.database, now()).await.map(|_| ())
    }
}

/// Một dòng gộp của một website, một chiều, một giá trị.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    pub views: i64,
    pub visitors: i64,
    pub events: i64,
    pub bounces: i64,
}

type Rows = BTreeMap<(Uuid, &'static str, String), Counts>;

impl AnalyticsService {
    /// Gộp mọi khách có sự kiện thô trên `database`; trả số ngày đã gộp.
    pub(crate) async fn rollup_database(
        &self,
        database: &Database,
        now: Timestamp,
    ) -> Result<usize, Error> {
        let tenants: Vec<Uuid> = event::Entity::find()
            .select_only()
            .column(event::Column::TenantId)
            .distinct()
            .into_tuple()
            .all(database.read())
            .await?;
        let mut rolled = 0;
        for id in tenants {
            match self.registry.tenant(TenantId::from_uuid(id)).await {
                Ok(tenant) => rolled += self.rollup_tenant(&tenant, now).await?,
                Err(error) => {
                    tracing::warn!(tenant_id = %id, error = %error, "analytics rollup skipped a tenant");
                }
            }
        }
        Ok(rolled)
    }

    /// Gộp các ngày cần gộp của khách `tenant`; trả số ngày.
    pub(crate) async fn rollup_tenant(
        &self,
        tenant: &TenantContext,
        now: Timestamp,
    ) -> Result<usize, Error> {
        let read = tenant.database().read();
        let tenant_id = tenant.id().as_uuid();
        let raw_days: BTreeSet<String> = event::Entity::find()
            .select_only()
            .column(event::Column::Day)
            .distinct()
            .filter(event::Column::TenantId.eq(tenant_id))
            .into_tuple::<String>()
            .all(read)
            .await?
            .into_iter()
            .collect();
        let done: BTreeSet<String> = rollup::Entity::find()
            .select_only()
            .column(rollup::Column::Day)
            .filter(rollup::Column::TenantId.eq(tenant_id))
            .into_tuple::<String>()
            .all(read)
            .await?
            .into_iter()
            .collect();
        let recent = day_of(now - SignedDuration::from_hours(24 * RECENT_DAYS));
        let days: Vec<String> =
            raw_days.into_iter().filter(|day| *day >= recent || !done.contains(day)).collect();
        for day in &days {
            self.rollup_day(tenant, day, now).await?;
        }
        Ok(days.len())
    }

    /// Tính lại số gộp của ngày `day` cho khách `tenant`.
    pub(crate) async fn rollup_day(
        &self,
        tenant: &TenantContext,
        day: &str,
        now: Timestamp,
    ) -> Result<(), Error> {
        let read = tenant.database().read();
        let tenant_id = tenant.id().as_uuid();
        let mut rows: Rows = BTreeMap::new();
        let base = || {
            Query::select()
                .from(event::Entity)
                .and_where(Expr::col(event::Column::TenantId).eq(tenant_id))
                .and_where(Expr::col(event::Column::Day).eq(day))
                .to_owned()
        };
        let page_views = || Expr::col(event::Column::Kind).eq(PAGE_VIEW);
        // Phiên trình duyệt (phiên phía máy chủ bắt đầu bằng `~`).
        let browser = || Expr::col(event::Column::SessionKey).not_like("~%");

        // Tổng: mọi sự kiện, khách, lượt xem.
        let mut total = base();
        total
            .column(event::Column::SiteOrgId)
            .expr_as(Expr::val(""), Alias::new("value"))
            .expr_as(Func::count(Expr::col(event::Column::Id)), Alias::new("events"))
            .expr_as(sum_if(page_views()), Alias::new("views"))
            .group_by_col(event::Column::SiteOrgId);
        for (site, value, counts) in grouped(read, &total).await? {
            let entry = rows.entry((site, "total", value)).or_default();
            entry.events = counts.events;
            entry.views = counts.views;
        }
        let mut visitors = base();
        visitors
            .column(event::Column::SiteOrgId)
            .expr_as(Expr::val(""), Alias::new("value"))
            .expr_as(
                Func::count_distinct(Expr::col(event::Column::SessionKey)),
                Alias::new("visitors"),
            )
            .and_where(browser())
            .group_by_col(event::Column::SiteOrgId);
        for (site, value, counts) in grouped(read, &visitors).await? {
            rows.entry((site, "total", value)).or_default().visitors = counts.visitors;
        }
        // Phiên thoát: phiên chỉ xem đúng một trang.
        let mut single = base();
        single
            .column(event::Column::SiteOrgId)
            .column(event::Column::SessionKey)
            .and_where(page_views())
            .group_by_col(event::Column::SiteOrgId)
            .group_by_col(event::Column::SessionKey)
            .and_having(Func::count(Expr::col(event::Column::Id)).eq(1));
        let bounces = Query::select()
            .column(event::Column::SiteOrgId)
            .expr_as(Expr::val(""), Alias::new("value"))
            .expr_as(Func::count(Expr::col(event::Column::SessionKey)), Alias::new("bounces"))
            .from_subquery(single, Alias::new("single"))
            .group_by_col(event::Column::SiteOrgId)
            .to_owned();
        for (site, value, counts) in grouped(read, &bounces).await? {
            rows.entry((site, "total", value)).or_default().bounces = counts.bounces;
        }

        // Trang, nguồn, chiến dịch: chỉ lượt xem trang.
        for (dimension, column, landing_only) in [
            ("page", event::Column::Path, false),
            ("source", event::Column::Source, true),
            ("campaign", event::Column::UtmCampaign, true),
        ] {
            let mut select = base();
            select
                .column(event::Column::SiteOrgId)
                .expr_as(Expr::col(column), Alias::new("value"))
                .expr_as(Func::count(Expr::col(event::Column::Id)), Alias::new("views"))
                .expr_as(
                    Func::count_distinct(Expr::col(event::Column::SessionKey)),
                    Alias::new("visitors"),
                )
                .and_where(page_views())
                .and_where(Expr::col(column).is_not_null())
                .group_by_col(event::Column::SiteOrgId)
                .group_by_col(column);
            if landing_only {
                select.and_where(Expr::col(event::Column::Landing).eq(true));
            }
            for (site, value, counts) in grouped(read, &select).await? {
                rows.insert((site, dimension, value), counts);
            }
        }
        // Đối tượng và loại sự kiện: mọi sự kiện.
        for (dimension, column) in
            [("object", event::Column::Object), ("type", event::Column::Kind)]
        {
            let mut select = base();
            select
                .column(event::Column::SiteOrgId)
                .expr_as(Expr::col(column), Alias::new("value"))
                .expr_as(Func::count(Expr::col(event::Column::Id)), Alias::new("events"))
                .expr_as(
                    Func::count_distinct(Expr::col(event::Column::SessionKey)),
                    Alias::new("visitors"),
                )
                .and_where(Expr::col(column).is_not_null())
                .group_by_col(event::Column::SiteOrgId)
                .group_by_col(column);
            for (site, value, mut counts) in grouped(read, &select).await? {
                // Lượt xem của một đối tượng là số sự kiện nói tới nó.
                if dimension == "object" {
                    counts.views = counts.events;
                }
                rows.insert((site, dimension, value), counts);
            }
        }

        // Phễu: mỗi bước, số phiên có sự kiện của bước.
        let mut steps = Vec::new();
        for funnel in self.catalog.funnels() {
            for (index, step) in funnel.steps.iter().enumerate() {
                let mut select = base();
                select
                    .column(event::Column::SiteOrgId)
                    .expr_as(Expr::val(""), Alias::new("value"))
                    .expr_as(
                        Func::count_distinct(Expr::col(event::Column::SessionKey)),
                        Alias::new("visitors"),
                    )
                    .and_where(Expr::col(event::Column::Kind).eq(*step))
                    .group_by_col(event::Column::SiteOrgId);
                for (site, _, counts) in grouped(read, &select).await? {
                    steps.push(funnel_daily::ActiveModel {
                        id: Set(Uuid::now_v7()),
                        tenant_id: Set(tenant_id),
                        site_org_id: Set(site),
                        day: Set(day.to_owned()),
                        funnel: Set(funnel.key.to_owned()),
                        step: Set(i32::try_from(index).unwrap_or(i32::MAX)),
                        step_key: Set((*step).to_owned()),
                        sessions: Set(counts.visitors),
                    });
                }
            }
        }

        let transaction = tenant.database().begin_write().await?;
        daily::Entity::delete_many()
            .filter(daily::Column::TenantId.eq(tenant_id))
            .filter(daily::Column::Day.eq(day))
            .exec(&transaction)
            .await?;
        funnel_daily::Entity::delete_many()
            .filter(funnel_daily::Column::TenantId.eq(tenant_id))
            .filter(funnel_daily::Column::Day.eq(day))
            .exec(&transaction)
            .await?;
        let models: Vec<daily::ActiveModel> = rows
            .into_iter()
            .map(|((site, dimension, value), counts)| daily::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant_id),
                site_org_id: Set(site),
                day: Set(day.to_owned()),
                dimension: Set(dimension.to_owned()),
                value: Set(value),
                views: Set(counts.views),
                visitors: Set(counts.visitors),
                events: Set(counts.events),
                bounces: Set(counts.bounces),
            })
            .collect();
        for chunk in models.chunks(CHUNK) {
            daily::Entity::insert_many(chunk.to_vec()).exec_without_returning(&transaction).await?;
        }
        for chunk in steps.chunks(CHUNK) {
            funnel_daily::Entity::insert_many(chunk.to_vec())
                .exec_without_returning(&transaction)
                .await?;
        }
        rollup::Entity::delete_many()
            .filter(rollup::Column::TenantId.eq(tenant_id))
            .filter(rollup::Column::Day.eq(day))
            .exec(&transaction)
            .await?;
        rollup::Entity::insert(rollup::ActiveModel {
            tenant_id: Set(tenant_id),
            day: Set(day.to_owned()),
            rolled_at: Set(now),
        })
        .exec_without_returning(&transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }
}

/// `SUM(CASE WHEN điều kiện THEN 1 ELSE 0 END)`.
fn sum_if(condition: SimpleExpr) -> SimpleExpr {
    // Ép về BIGINT: hai database trả cùng kiểu số nguyên.
    Func::cast_as(Func::sum(Expr::case(condition, 1).finally(0)), Alias::new("BIGINT")).into()
}

/// Chạy `select` (cột `site_org_id`, `value` và các cột đếm có mặt) và đọc từng dòng; cột đếm vắng
/// mặt là 0.
async fn grouped<C: ConnectionTrait>(
    connection: &C,
    select: &sea_orm::sea_query::SelectStatement,
) -> Result<Vec<(Uuid, String, Counts)>, Error> {
    let rows = connection.query_all(select).await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let count = |name: &str| -> i64 {
            // Cột vắng mặt trong câu này → 0. SUM trên Postgres trả `numeric` / `bigint`: đọc i64.
            row.try_get::<Option<i64>>("", name).ok().flatten().unwrap_or(0)
        };
        let value: Option<String> = row.try_get("", "value")?;
        let Some(value) = value else { continue };
        out.push((
            row.try_get::<Uuid>("", "site_org_id")?,
            value,
            Counts {
                views: count("views"),
                visitors: count("visitors"),
                events: count("events"),
                bounces: count("bounces"),
            },
        ));
    }
    Ok(out)
}
