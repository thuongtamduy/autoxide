//! Bán chạy, điểm đánh giá (D-201) và "thường mua cùng" (B-c14), dựng từ event của `orders`.
//!
//! - `orders.order_completed` → một dòng `catalog_sold_products` mỗi (đơn, sản phẩm); unique theo
//!   đơn nên nhận lại event không đếm hai lần. Rồi **tính lại từ đầu** cho các sản phẩm của đơn:
//!   số bán trong [`WINDOW_DAYS`] ngày (`catalog_product_stats.units_sold` → cột `best_selling`
//!   của storefront) và top [`MAX_PAIRS`] sản phẩm cùng đơn (`catalog_co_purchases`).
//! - Đánh giá (`orders.review_published` / `review_moderated`): payload mang **tổng** số đánh giá
//!   và tổng số sao của sản phẩm — ghi đè, nên nhận lại hay nhận lệch thứ tự vẫn đúng.
//! - Lịch mỗi ngày [`REFRESH_SALES`]: bỏ dòng bán cũ hơn cửa sổ và tính lại sản phẩm bị ảnh hưởng —
//!   sản phẩm lâu không bán tụt hạng dù không có đơn mới.
//!
//! Mọi câu đếm dùng index bắt đầu bằng `tenant_id`, chạy như nhau trên SQLite và Postgres.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::job::{JobContext, JobHandler};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext, TenantId};
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Func, JoinType, OnConflict, Order, Query};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QuerySelect, Set};
use uuid::Uuid;

use crate::entity::{co_purchase, product_category, product_stat, sold_product, storefront_entry};
use crate::service::{CatalogService, now};

/// Cửa sổ đếm bán chạy và mua cùng.
pub(crate) const WINDOW_DAYS: i64 = 90;
/// Số sản phẩm "thường mua cùng" giữ cho mỗi sản phẩm.
pub(crate) const MAX_PAIRS: u64 = 20;
/// Số sản phẩm tối đa xét trong một đơn (đơn sỉ rất dài không làm job chạy mãi).
const MAX_ORDER_PRODUCTS: usize = 100;
/// Lịch mỗi ngày: bỏ dòng bán cũ, tính lại.
pub(crate) const REFRESH_SALES: &str = "catalog.refresh_sales";

pub(crate) use crate::api::events::{SoldLine, completed_order, product_rating};

/// Điểm trung bình × 100, làm tròn; 0 khi chưa có đánh giá.
fn rating_x100(count: i64, sum: i64) -> i32 {
    if count <= 0 {
        return 0;
    }
    let value = (sum.saturating_mul(200) + count) / (count * 2);
    i32::try_from(value).unwrap_or(500)
}

fn cast_bigint(expr: impl Into<Expr>) -> Expr {
    Func::cast_as(expr.into(), Alias::new("BIGINT")).into()
}

impl CatalogService {
    /// Ghi các sản phẩm của đơn `order_id` hoàn tất lúc `at`, rồi tính lại số bán và mua cùng.
    pub(crate) async fn record_completed_order(
        &self,
        tenant: &TenantContext,
        order_id: Uuid,
        lines: &[SoldLine],
        at: Timestamp,
    ) -> Result<(), Error> {
        let mut quantities: BTreeMap<Uuid, i64> = BTreeMap::new();
        for line in lines {
            let total = quantities.entry(line.product_id).or_default();
            *total = total.saturating_add(line.quantity);
        }
        let products: Vec<Uuid> = quantities.keys().copied().take(MAX_ORDER_PRODUCTS).collect();
        if products.is_empty() {
            return Ok(());
        }
        let transaction = tenant.database().begin_write().await?;
        for product in &products {
            let row = sold_product::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                order_id: Set(order_id),
                product_id: Set(*product),
                quantity: Set(quantities[product]),
                completed_at: Set(at),
            };
            // Nhận lại event: dòng đã có, bỏ qua (không đếm hai lần).
            sold_product::Entity::insert(row)
                .on_conflict(
                    OnConflict::columns([
                        sold_product::Column::TenantId,
                        sold_product::Column::OrderId,
                        sold_product::Column::ProductId,
                    ])
                    .do_nothing()
                    .to_owned(),
                )
                .exec_without_returning(&transaction)
                .await?;
        }
        transaction.commit().await?;
        self.refresh_sales(tenant, &products).await
    }

    /// Tính lại số bán trong cửa sổ và "thường mua cùng" của `products`, từ đầu (chạy lại được).
    pub(crate) async fn refresh_sales(
        &self,
        tenant: &TenantContext,
        products: &[Uuid],
    ) -> Result<(), Error> {
        let since = now() - SignedDuration::from_hours(24 * WINDOW_DAYS);
        for product in products {
            let units = self.units_sold(tenant, *product, since).await?;
            let pairs = self.co_purchased(tenant, *product, since).await?;
            let at = now();
            let transaction = tenant.database().begin_write().await?;
            let stat = product_stat::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                product_id: Set(*product),
                units_sold: Set(units),
                rating_count: Set(0),
                rating_sum: Set(0),
                updated_at: Set(at),
            };
            product_stat::Entity::insert(stat)
                .on_conflict(
                    OnConflict::columns([
                        product_stat::Column::TenantId,
                        product_stat::Column::ProductId,
                    ])
                    .update_columns([
                        product_stat::Column::UnitsSold,
                        product_stat::Column::UpdatedAt,
                    ])
                    .to_owned(),
                )
                .exec_without_returning(&transaction)
                .await?;
            storefront_entry::Entity::update_many()
                .col_expr(storefront_entry::Column::BestSelling, Expr::value(units))
                .filter(storefront_entry::Column::ProductId.eq(*product))
                .scoped(tenant)
                .exec(&transaction)
                .await?;
            co_purchase::Entity::delete_many()
                .filter(co_purchase::Column::ProductId.eq(*product))
                .scoped(tenant)
                .exec(&transaction)
                .await?;
            if !pairs.is_empty() {
                let rows = pairs.iter().map(|(other, count)| co_purchase::ActiveModel {
                    id: Set(Uuid::now_v7()),
                    tenant_id: Set(tenant.id().as_uuid()),
                    product_id: Set(*product),
                    other_product_id: Set(*other),
                    orders_count: Set(*count),
                    updated_at: Set(at),
                });
                co_purchase::Entity::insert_many(rows).exec_without_returning(&transaction).await?;
            }
            transaction.commit().await?;
        }
        Ok(())
    }

    async fn units_sold(
        &self,
        tenant: &TenantContext,
        product: Uuid,
        since: Timestamp,
    ) -> Result<i64, Error> {
        let mut select = Query::select();
        select
            .from(sold_product::Entity)
            .expr_as(
                cast_bigint(Func::coalesce([
                    Func::sum(Expr::col(sold_product::Column::Quantity)).into(),
                    Expr::value(0),
                ])),
                Alias::new("units"),
            )
            .and_where(Expr::col(sold_product::Column::TenantId).eq(tenant.id().as_uuid()))
            .and_where(Expr::col(sold_product::Column::ProductId).eq(product))
            .and_where(Expr::col(sold_product::Column::CompletedAt).gte(since));
        let row = tenant.database().read().query_one(&select).await?;
        Ok(match row {
            Some(row) => row.try_get::<Option<i64>>("", "units")?.unwrap_or(0),
            None => 0,
        })
    }

    /// Top [`MAX_PAIRS`] sản phẩm cùng đơn với `product` trong cửa sổ: (sản phẩm, số đơn).
    async fn co_purchased(
        &self,
        tenant: &TenantContext,
        product: Uuid,
        since: Timestamp,
    ) -> Result<Vec<(Uuid, i64)>, Error> {
        let (mine, other) = (Alias::new("mine"), Alias::new("other"));
        let count =
            cast_bigint(Func::count(Expr::col((other.clone(), sold_product::Column::OrderId))));
        let mut select = Query::select();
        select
            .from_as(sold_product::Entity, mine.clone())
            .join_as(
                JoinType::InnerJoin,
                sold_product::Entity,
                other.clone(),
                Expr::col((other.clone(), sold_product::Column::TenantId))
                    .equals((mine.clone(), sold_product::Column::TenantId))
                    .and(
                        Expr::col((other.clone(), sold_product::Column::OrderId))
                            .equals((mine.clone(), sold_product::Column::OrderId)),
                    ),
            )
            .expr_as(
                Expr::col((other.clone(), sold_product::Column::ProductId)),
                Alias::new("product_id"),
            )
            .expr_as(count.clone(), Alias::new("orders"))
            .and_where(
                Expr::col((mine.clone(), sold_product::Column::TenantId)).eq(tenant.id().as_uuid()),
            )
            .and_where(Expr::col((mine.clone(), sold_product::Column::ProductId)).eq(product))
            .and_where(Expr::col((mine, sold_product::Column::CompletedAt)).gte(since))
            .and_where(Expr::col((other.clone(), sold_product::Column::ProductId)).ne(product))
            .group_by_col((other.clone(), sold_product::Column::ProductId))
            .order_by_expr(count, Order::Desc)
            .order_by((other, sold_product::Column::ProductId), Order::Asc)
            .limit(MAX_PAIRS);
        let rows = tenant.database().read().query_all(&select).await?;
        let mut pairs = Vec::with_capacity(rows.len());
        for row in rows {
            pairs.push((row.try_get("", "product_id")?, row.try_get::<i64>("", "orders")?));
        }
        Ok(pairs)
    }

    /// Ghi tổng đánh giá của `product` (ghi đè) và điểm trên storefront.
    pub(crate) async fn record_rating(
        &self,
        tenant: &TenantContext,
        product: Uuid,
        count: i64,
        sum: i64,
    ) -> Result<(), Error> {
        let transaction = tenant.database().begin_write().await?;
        let stat = product_stat::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            product_id: Set(product),
            units_sold: Set(0),
            rating_count: Set(count),
            rating_sum: Set(sum),
            updated_at: Set(now()),
        };
        product_stat::Entity::insert(stat)
            .on_conflict(
                OnConflict::columns([
                    product_stat::Column::TenantId,
                    product_stat::Column::ProductId,
                ])
                .update_columns([
                    product_stat::Column::RatingCount,
                    product_stat::Column::RatingSum,
                    product_stat::Column::UpdatedAt,
                ])
                .to_owned(),
            )
            .exec_without_returning(&transaction)
            .await?;
        storefront_entry::Entity::update_many()
            .col_expr(storefront_entry::Column::RatingX100, Expr::value(rating_x100(count, sum)))
            .col_expr(storefront_entry::Column::RatingCount, Expr::value(count))
            .filter(storefront_entry::Column::ProductId.eq(product))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Số bán và điểm của `products`, để dòng storefront mới dựng có ngay (không chờ đơn sau).
    pub(crate) async fn product_stats<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        products: &[Uuid],
    ) -> Result<BTreeMap<Uuid, (i64, i32, i64)>, Error> {
        let rows = product_stat::Entity::find()
            .filter(product_stat::Column::ProductId.is_in(products.iter().copied()))
            .scoped(tenant)
            .all(connection)
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let score = rating_x100(row.rating_count, row.rating_sum);
                (row.product_id, (row.units_sold, score, row.rating_count))
            })
            .collect())
    }

    /// Sản phẩm "thường mua cùng" `product`, nhiều đơn nhất trước.
    pub(crate) async fn bought_together(
        &self,
        tenant: &TenantContext,
        product: Uuid,
    ) -> Result<Vec<Uuid>, Error> {
        use sea_orm::QueryOrder as _;
        Ok(co_purchase::Entity::find()
            .select_only()
            .column(co_purchase::Column::OtherProductId)
            .filter(co_purchase::Column::ProductId.eq(product))
            .scoped(tenant)
            .order_by_desc(co_purchase::Column::OrdersCount)
            .order_by_asc(co_purchase::Column::OtherProductId)
            .limit(MAX_PAIRS)
            .into_tuple()
            .all(tenant.database().read())
            .await?)
    }
}

/// Lịch mỗi ngày: bỏ dòng bán cũ hơn cửa sổ (theo từng khách trên database này), tính lại sản phẩm
/// có dòng vừa bỏ.
pub(crate) struct RefreshSales(pub(crate) Arc<CatalogService>);

#[async_trait::async_trait]
impl JobHandler for RefreshSales {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let since = now() - SignedDuration::from_hours(24 * WINDOW_DAYS);
        let rows: Vec<(Uuid, Uuid)> = sold_product::Entity::find()
            .select_only()
            .column(sold_product::Column::TenantId)
            .column(sold_product::Column::ProductId)
            .filter(sold_product::Column::CompletedAt.lt(since))
            .distinct()
            .into_tuple()
            .all(job.database.read())
            .await?;
        let mut by_tenant: BTreeMap<Uuid, BTreeSet<Uuid>> = BTreeMap::new();
        for (tenant, product) in rows {
            by_tenant.entry(tenant).or_default().insert(product);
        }
        for (tenant, products) in by_tenant {
            let tenant = self.0.databases.tenant(TenantId::from_uuid(tenant)).await?;
            let write = tenant.database().begin_write().await?;
            sold_product::Entity::delete_many()
                .filter(sold_product::Column::CompletedAt.lt(since))
                .scoped(&tenant)
                .exec(&write)
                .await?;
            write.commit().await?;
            let products: Vec<Uuid> = products.into_iter().collect();
            self.0.refresh_sales(&tenant, &products).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn diem_trung_binh_lam_tron() {
        assert_eq!(rating_x100(0, 0), 0);
        assert_eq!(rating_x100(3, 14), 467);
        assert_eq!(rating_x100(2, 9), 450);
        assert_eq!(rating_x100(1, 5), 500);
    }

    #[test]
    fn doc_payload_cua_orders() {
        let order = Uuid::now_v7();
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        let payload = json!({"after": {"order_id": order, "lines": [
            {"product_id": a, "quantity": 2},
            {"product_id": b, "quantity": 0},
            {"quantity": 1},
        ]}});
        let (found, lines) = completed_order(&payload).unwrap();
        assert_eq!(found, order);
        assert_eq!(lines, vec![SoldLine { product_id: a, quantity: 2 }]);
        assert!(completed_order(&json!({"after": {}})).is_none());

        let rating = json!({"after": {"product_id": a, "product_rating": {"count": 2, "sum": 9}}});
        assert_eq!(product_rating(&rating), Some((a, 2, 9)));
        let impossible =
            json!({"after": {"product_id": a, "product_rating": {"count": 1, "sum": 6}}});
        assert_eq!(product_rating(&impossible), None);
    }
}

/// Loại gợi ý trên trang sản phẩm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Recommendation {
    /// Sản phẩm tương tự theo ý nghĩa; chưa có thì cùng danh mục, bán chạy trước.
    Similar,
    /// Thường mua cùng (từ đơn hoàn tất trong 90 ngày).
    BoughtTogether,
}

impl CatalogService {
    /// Gợi ý cho trang của `product` trên website `site`, tối đa `limit` thẻ. `product` phải đang
    /// hiện ở website đó (không thì 404); gợi ý chỉ gồm sản phẩm đang hiện ở website đó.
    pub(crate) async fn recommendations(
        &self,
        tenant: &TenantContext,
        site: &autoxide_kernel::tenancy::Site,
        product: Uuid,
        locale: &str,
        kind: Recommendation,
        limit: usize,
    ) -> Result<Vec<crate::public::PublicCard>, Error> {
        let read = tenant.database().read();
        let visible = self.visible_on(tenant, site).await?;
        let shown = storefront_entry::Entity::find()
            .filter(visible.clone())
            .filter(storefront_entry::Column::ProductId.eq(product))
            .scoped(tenant)
            .one(read)
            .await?;
        if shown.is_none() {
            return Err(crate::errors::product_not_found(product));
        }
        let ids = match kind {
            Recommendation::BoughtTogether => self.bought_together(tenant, product).await?,
            Recommendation::Similar => {
                let found = self.similar_products(tenant, product, locale).await?;
                if found.is_empty() {
                    return self.same_category(tenant, visible, product, locale, limit).await;
                }
                found
            }
        };
        let mut entries: HashMap<Uuid, storefront_entry::Model> = storefront_entry::Entity::find()
            .filter(visible)
            .filter(storefront_entry::Column::ProductId.is_in(ids.clone()))
            .scoped(tenant)
            .all(read)
            .await?
            .into_iter()
            .map(|row| (row.product_id, row))
            .collect();
        let ordered: Vec<storefront_entry::Model> =
            ids.iter().filter_map(|id| entries.remove(id)).take(limit).collect();
        self.cards(tenant, locale, ordered).await
    }

    /// Sản phẩm cùng danh mục với `product` (bán chạy trước): khi chưa có danh sách tương tự.
    async fn same_category(
        &self,
        tenant: &TenantContext,
        visible: sea_orm::Condition,
        product: Uuid,
        locale: &str,
        limit: usize,
    ) -> Result<Vec<crate::public::PublicCard>, Error> {
        use sea_orm::QueryOrder as _;
        let read = tenant.database().read();
        let categories: Vec<Uuid> = product_category::Entity::find()
            .select_only()
            .column(product_category::Column::CategoryId)
            .filter(product_category::Column::ProductId.eq(product))
            .scoped(tenant)
            .into_tuple()
            .all(read)
            .await?;
        if categories.is_empty() {
            return Ok(Vec::new());
        }
        let members = Query::select()
            .column(product_category::Column::ProductId)
            .from(product_category::Entity)
            .and_where(Expr::col(product_category::Column::TenantId).eq(tenant.id().as_uuid()))
            .and_where(Expr::col(product_category::Column::CategoryId).is_in(categories))
            .to_owned();
        let entries = storefront_entry::Entity::find()
            .filter(visible)
            .filter(storefront_entry::Column::ProductId.in_subquery(members))
            .filter(storefront_entry::Column::ProductId.ne(product))
            .scoped(tenant)
            .order_by_desc(storefront_entry::Column::BestSelling)
            .order_by_asc(storefront_entry::Column::ProductId)
            .limit(u64::try_from(limit).unwrap_or(u64::MAX))
            .all(read)
            .await?;
        self.cards(tenant, locale, entries).await
    }
}
