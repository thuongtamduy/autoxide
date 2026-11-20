//! Thống kê công khai của đánh giá một sản phẩm: sao, có ảnh, theo biến thể, bản tóm tắt AI đã duyệt
//! (website bán hàng hiện sẵn — D-109: tóm tắt AI chỉ hiện khi người duyệt).

use sea_orm::sea_query::{Alias, Expr, Func};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use uuid::Uuid;

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};

use crate::entity::{review, review_stat, review_summary};
use crate::reviews::PUBLISHED;
use crate::service::OrdersService;

/// Thống kê công khai của một sản phẩm.
#[derive(Debug, Clone, Default)]
pub(crate) struct Stats {
    pub count: i64,
    pub rating_sum: i64,
    pub stars: [i64; 5],
    pub with_media: i64,
    /// Số đánh giá và tổng sao theo biến thể (biến thể, số, tổng sao).
    pub by_variant: Vec<(Uuid, i64, i64)>,
    pub summary: Option<review_summary::Model>,
}

impl OrdersService {
    /// Thống kê sao và bản tóm tắt AI đã duyệt của sản phẩm (`locale`).
    pub(crate) async fn review_stats(
        &self,
        tenant: &TenantContext,
        product_id: Uuid,
        locale: &str,
    ) -> Result<Stats, Error> {
        let read = tenant.database().read();
        let row = review_stat::Entity::find()
            .filter(review_stat::Column::ProductId.eq(product_id))
            .scoped(tenant)
            .one(read)
            .await?;
        let summary = review_summary::Entity::find()
            .filter(review_summary::Column::ApprovedKey.eq(format!("{product_id}:{locale}")))
            .scoped(tenant)
            .one(read)
            .await?;
        // Theo biến thể: một câu `GROUP BY` trên index (tenant, product, status, …).
        let by_variant: Vec<(Uuid, i64, Option<i64>)> = review::Entity::find()
            .select_only()
            .column(review::Column::VariantId)
            .column_as(review::Column::Id.count(), "reviews")
            .column_as(
                Expr::from(Func::cast_as(
                    Func::sum(Expr::col(review::Column::Rating)),
                    Alias::new("BIGINT"),
                )),
                "rating_sum",
            )
            .filter(review::Column::ProductId.eq(product_id))
            .filter(review::Column::Status.eq(PUBLISHED))
            .scoped(tenant)
            .group_by(review::Column::VariantId)
            .order_by_asc(review::Column::VariantId)
            .into_tuple()
            .all(read)
            .await?;
        let by_variant = by_variant
            .into_iter()
            .map(|(variant, count, sum)| (variant, count, sum.unwrap_or(0)))
            .collect();
        Ok(match row {
            Some(row) => Stats {
                count: row.review_count,
                rating_sum: row.rating_sum,
                stars: [row.count_1, row.count_2, row.count_3, row.count_4, row.count_5],
                with_media: row.media_count,
                by_variant,
                summary,
            },
            None => Stats { summary, by_variant, ..Stats::default() },
        })
    }
}
