//! Mã công khai áp được cho một sản phẩm (D-200): trang sản phẩm hiện "còn X với mã …" — giá niêm
//! yết giữ nguyên (Luật 19/2023: không trình bày giá có mã như giá đã giảm sẵn).

use autoxide_catalog::api::PriceQuery;
use autoxide_kernel::error::Error;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::tenancy::{ScopeExt, Site, TenantContext};
use autoxide_kernel::time::Timestamp;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::api::{Cart, CartLine};
use crate::entity::{code, promotion, translation};
use crate::evaluate::Options;
use crate::model::{Purpose, Status, Trigger};
use crate::service::PromotionsService;

/// Số mã xét mỗi chương trình (như bộ chọn mã của giỏ).
const CODES_PER_PROGRAM: u64 = 3;

/// A public coupon that lowers the price of a product.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = PromotionsProductCode)]
pub(crate) struct ProductCode {
    /// The coupon to enter at checkout.
    pub code: String,
    pub promotion_id: Uuid,
    pub title: String,
    /// Conditions, in full (the same as in `GET /public/promotions`).
    pub terms: String,
    /// Price of one unit with the coupon. The listed price stays the price without it: show
    /// this as "X with code …", not as a discounted price.
    pub price_with_code: Money,
    /// Discount on one unit.
    pub discount: Money,
    /// Uses left, only when the coupon or promotion has a real limit; `null` otherwise.
    pub uses_left: Option<i64>,
    pub ends_at: Timestamp,
}

/// Biến thể không thuộc sản phẩm (hay không có) — một lỗi cho cả hai, không lộ biến thể của ai.
fn product_not_found(product: Uuid) -> Error {
    Error::not_found(
        "promotions.product_not_found",
        "There is no product `{product}` with this variant on sale here.",
    )
    .with_param("product", product)
}

impl PromotionsService {
    /// Mã `public` đang chạy áp được cho biến thể `variant_id` của sản phẩm `product_id` trên
    /// website `site`, mỗi mã một giá sau mã cho số lượng 1. Mã không giảm thêm gì thì vắng.
    pub(crate) async fn codes_for_product(
        &self,
        tenant: &TenantContext,
        site: &Site,
        product_id: Uuid,
        variant_id: Uuid,
        locale: &str,
    ) -> Result<Vec<ProductCode>, Error> {
        let found = self.catalog.variants(tenant, &[variant_id], None).await?;
        if !found.iter().any(|variant| variant.product_id == product_id) {
            return Err(product_not_found(product_id));
        }
        let read = tenant.database().read();
        let now = crate::service::now();
        let programs = promotion::Entity::find()
            .filter(promotion::Column::Status.eq(Status::Active.as_str()))
            .filter(promotion::Column::Public.eq(true))
            .filter(promotion::Column::Trigger.eq(Trigger::Code.as_str()))
            .filter(promotion::Column::OrgId.is_in(site.lineage_ids()))
            .filter(promotion::Column::StartsAt.lte(now))
            .filter(promotion::Column::EndsAt.gt(now))
            .scoped(tenant)
            .order_by_asc(promotion::Column::Id)
            .all(read)
            .await?;
        let ids: Vec<Uuid> = programs.iter().map(|row| row.id).collect();
        let texts = translation::Entity::find()
            .filter(translation::Column::PromotionId.is_in(ids))
            .scoped(tenant)
            .order_by_asc(translation::Column::Id)
            .all(read)
            .await?;
        let mut out = Vec::new();
        for program in &programs {
            let Ok(currency) = Currency::parse(&program.currency) else { continue };
            let query = PriceQuery::new(site.org(), currency, now, vec![variant_id]);
            let Some(price) = self.catalog.prices(tenant, &query).await?.into_iter().next() else {
                continue;
            };
            let mut cart = Cart::new(site.org(), currency, now);
            let mut line = CartLine::new("item".to_owned(), variant_id, 1, price.amount);
            line.has_compare_at = price.compare_at.is_some();
            cart.lines.push(line);
            let baseline = self.evaluate_cart(tenant, &cart, &Options::default()).await?.evaluation;
            let codes = code::Entity::find()
                .filter(code::Column::PromotionId.eq(program.id))
                .filter(code::Column::Purpose.eq(Purpose::Redeem.as_str()))
                .scoped(tenant)
                .order_by_asc(code::Column::Id)
                .limit(CODES_PER_PROGRAM)
                .all(read)
                .await?;
            for found in codes {
                let mut trial = cart.clone();
                trial.codes = vec![found.code.clone()];
                let result = self.evaluate_cart(tenant, &trial, &Options::default()).await?;
                let extra =
                    result.evaluation.total_discount.minor() - baseline.total_discount.minor();
                let applied = result.applied.iter().any(|(applied, _)| applied.id == program.id);
                if extra <= 0 || !applied {
                    continue;
                }
                // Số lượt còn chỉ khi có giới hạn thật (Luật 19/2023: số còn là số thật).
                let left = |limit: Option<i64>, used: i64| limit.map(|limit| (limit - used).max(0));
                let uses_left = match (
                    left(found.usage_limit, found.usage_count),
                    left(program.usage_limit, program.usage_count),
                ) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                };
                let text = texts
                    .iter()
                    .find(|text| text.promotion_id == program.id && text.locale == locale)
                    .or_else(|| texts.iter().find(|text| text.promotion_id == program.id));
                out.push(ProductCode {
                    code: found.code,
                    promotion_id: program.id,
                    title: text.map(|text| text.title.clone()).unwrap_or_default(),
                    terms: text.map(|text| text.terms.clone()).unwrap_or_default(),
                    price_with_code: Money::new(price.amount.minor() - extra, currency),
                    discount: Money::new(extra, currency),
                    uses_left,
                    ends_at: program.ends_at,
                });
            }
        }
        // Rẻ nhất trước: mã giảm nhiều nhất là dòng người mua đọc đầu tiên.
        out.sort_by_key(|row| (row.price_with_code.minor(), row.code.clone()));
        Ok(out)
    }
}
