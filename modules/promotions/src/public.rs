//! Danh sách chương trình công khai cho website bán hàng (Luật 19/2023: điều kiện khuyến mại rõ
//! ràng). Mỗi chương trình kèm số tiếp nhận thông báo đã ghi nhận với Sở Công Thương và mức giảm
//! tối đa theo luật đang áp hôm nay (dữ liệu có ngày hiệu lực, luật số hai).

use autoxide_kernel::error::Error;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{ScopeExt, Site, TenantContext};
use chrono::{DateTime, Utc};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::compliance::legal_day;
use crate::entity::{flash_item, notice, promotion, translation};
use crate::legal::legal_kind;
use crate::model::{Kind, Status};
use crate::service::PromotionsService;

/// A running promotion, as buyers see it.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = PromotionsPublicPromotion)]
pub(crate) struct PublicPromotion {
    pub id: Uuid,
    /// Unit that runs it.
    pub org_id: Uuid,
    /// Kind of the promotion (`percent_off`, `flash_sale`…).
    pub kind: String,
    /// `automatic`, `code` or `voucher`.
    pub trigger: String,
    /// Language of `title` and `terms` (the one asked for, else the first the promotion has).
    pub locale: String,
    pub title: String,
    /// Conditions, in full.
    pub terms: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    /// Discount in basis points (1000 = 10%), when the promotion has one.
    pub value_bp: Option<i32>,
    /// Discount amount in minor units of `currency`, when the promotion has one.
    pub value_minor: Option<i64>,
    /// Smallest order, in minor units of `currency`.
    pub min_order_minor: Option<i64>,
    pub currency: String,
    /// Only for the buyer's first order.
    pub first_order_only: bool,
    /// Buyers can save a voucher of it (`POST /public/promotions/{id}/claim`).
    pub claimable: bool,
    /// Notices recorded with the authorities (e.g. `province:79`, the Department of Industry and
    /// Trade of that province), with their reference.
    pub notices: Vec<PublicNotice>,
    /// The largest discount the law in force today allows for this kind of promotion; `null`
    /// when no law data is loaded.
    pub legal_limit: Option<PublicLegalLimit>,
}

/// A promotion notice recorded with an authority.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = PromotionsPublicNotice)]
pub(crate) struct PublicNotice {
    pub authority_code: String,
    /// Reference given by the authority.
    pub reference: Option<String>,
}

/// The legal discount limit in force.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = PromotionsPublicLegalLimit)]
pub(crate) struct PublicLegalLimit {
    /// Kind of promotion under the law (`price_reduction`, `gift_value`…).
    pub promotion_kind: String,
    /// Largest discount, in basis points of the price just before the promotion (5000 = 50%).
    pub max_bp: i32,
    /// Legal basis, with its date in force.
    pub legal_basis: String,
}

/// A public flash sale, running or starting within a day.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = PromotionsPublicFlashSale)]
pub(crate) struct PublicFlashSale {
    pub id: Uuid,
    pub org_id: Uuid,
    /// `running` or `upcoming`.
    pub state: String,
    pub title: String,
    /// Conditions, in full.
    pub terms: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub currency: String,
    pub items: Vec<PublicFlashItem>,
}

/// One product of a flash sale.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = PromotionsPublicFlashItem)]
pub(crate) struct PublicFlashItem {
    pub variant_id: Uuid,
    /// Sale price of one unit, in minor units (or `value_bp` off the price).
    pub sale_price_minor: Option<i64>,
    /// Discount in basis points (1000 = 10%), when the sale is a percentage.
    pub value_bp: Option<i32>,
    /// Units left at the sale price: a real count, `null` when not limited.
    pub quantity_left: Option<i64>,
    /// Most units one buyer can get at the sale price.
    pub per_customer_quantity_limit: Option<i64>,
}

/// Flash sale bắt đầu trong khoảng này thì hiện là `upcoming`.
const FLASH_SALE_HORIZON_HOURS: i64 = 24;

impl PromotionsService {
    /// Chương trình `public` đang chạy của website `site` (của nút đó và các nút cấp trên), cũ
    /// trước. Không cần đăng nhập.
    pub(crate) async fn public_promotions(
        &self,
        tenant: &TenantContext,
        site: &Site,
        locale: &str,
        page: &PageRequest,
    ) -> Result<Page<PublicPromotion>, Error> {
        let read = tenant.database().read();
        let now = crate::service::now();
        let select = promotion::Entity::find()
            .filter(promotion::Column::Status.eq(Status::Active.as_str()))
            .filter(promotion::Column::Public.eq(true))
            .filter(promotion::Column::OrgId.is_in(site.lineage_ids()))
            .filter(promotion::Column::StartsAt.lte(now))
            .filter(promotion::Column::EndsAt.gt(now))
            .scoped(tenant);
        let found = fetch_page(select, promotion::Column::Id, |row| row.id, page, read).await?;
        let ids: Vec<Uuid> = found.data.iter().map(|row| row.id).collect();
        let texts = translation::Entity::find()
            .filter(translation::Column::PromotionId.is_in(ids.clone()))
            .scoped(tenant)
            .order_by_asc(translation::Column::Id)
            .all(read)
            .await?;
        let notices = notice::Entity::find()
            .filter(notice::Column::PromotionId.is_in(ids))
            .filter(notice::Column::Status.eq("submitted"))
            .scoped(tenant)
            .order_by_asc(notice::Column::AuthorityCode)
            .all(read)
            .await?;
        // Loại theo luật cần chương trình đã biên dịch: lấy từ bộ nhớ (chương trình đang chạy luôn
        // có ở đó), không đọc lại phần con.
        let compiled = self
            .compiled
            .get(tenant, std::time::Duration::from_secs(self.config.compiled_ttl_seconds))
            .await?;
        let legal = self.legal().await?;
        let today = legal_day(now);
        Ok(found.map(|row| {
            let text = texts
                .iter()
                .find(|text| text.promotion_id == row.id && text.locale == locale)
                .or_else(|| texts.iter().find(|text| text.promotion_id == row.id));
            let legal_limit = compiled.get(row.id).and_then(|program| {
                let kind = legal_kind(program);
                let limit =
                    legal.limit(&self.config.jurisdiction, kind, program.program_kind, today)?;
                Some(PublicLegalLimit {
                    promotion_kind: kind.to_owned(),
                    max_bp: limit.max_bp,
                    legal_basis: limit.legal_basis.clone(),
                })
            });
            PublicPromotion {
                id: row.id,
                org_id: row.org_id,
                kind: row.kind,
                trigger: row.trigger,
                locale: text.map(|text| text.locale.clone()).unwrap_or_default(),
                title: text.map(|text| text.title.clone()).unwrap_or_default(),
                terms: text.map(|text| text.terms.clone()).unwrap_or_default(),
                starts_at: row.starts_at,
                ends_at: row.ends_at,
                value_bp: row.value_bp,
                value_minor: row.value_minor,
                min_order_minor: row.min_order_minor,
                currency: row.currency,
                first_order_only: row.first_order_only,
                claimable: row.claimable,
                notices: notices
                    .iter()
                    .filter(|found| found.promotion_id == row.id)
                    .map(|found| PublicNotice {
                        authority_code: found.authority_code.clone(),
                        reference: found.reference.clone(),
                    })
                    .collect(),
                legal_limit,
            }
        }))
    }
}

impl PromotionsService {
    /// Flash sale `public` đang chạy hay sẽ bắt đầu trong 24 giờ của website `site`, cũ trước; suất
    /// còn là số thật (giới hạn trừ số đã giữ và đã bán).
    pub(crate) async fn public_flash_sales(
        &self,
        tenant: &TenantContext,
        site: &Site,
        locale: &str,
        page: &PageRequest,
    ) -> Result<Page<PublicFlashSale>, Error> {
        let read = tenant.database().read();
        let now = crate::service::now();
        let horizon = now + chrono::Duration::hours(FLASH_SALE_HORIZON_HOURS);
        let select = promotion::Entity::find()
            .filter(promotion::Column::Kind.eq(Kind::FlashSale.as_str()))
            .filter(
                promotion::Column::Status
                    .is_in([Status::Active.as_str(), Status::Scheduled.as_str()]),
            )
            .filter(promotion::Column::Public.eq(true))
            .filter(promotion::Column::OrgId.is_in(site.lineage_ids()))
            .filter(promotion::Column::StartsAt.lt(horizon))
            .filter(promotion::Column::EndsAt.gt(now))
            .scoped(tenant);
        let found = fetch_page(select, promotion::Column::Id, |row| row.id, page, read).await?;
        let ids: Vec<Uuid> = found.data.iter().map(|row| row.id).collect();
        let texts = translation::Entity::find()
            .filter(translation::Column::PromotionId.is_in(ids.clone()))
            .scoped(tenant)
            .order_by_asc(translation::Column::Id)
            .all(read)
            .await?;
        let items = flash_item::Entity::find()
            .filter(flash_item::Column::PromotionId.is_in(ids))
            .scoped(tenant)
            .order_by_asc(flash_item::Column::Id)
            .all(read)
            .await?;
        Ok(found.map(|row| {
            let text = texts
                .iter()
                .find(|text| text.promotion_id == row.id && text.locale == locale)
                .or_else(|| texts.iter().find(|text| text.promotion_id == row.id));
            let running = row.status == Status::Active.as_str() && row.starts_at <= now;
            PublicFlashSale {
                id: row.id,
                org_id: row.org_id,
                state: if running { "running" } else { "upcoming" }.to_owned(),
                title: text.map(|text| text.title.clone()).unwrap_or_default(),
                terms: text.map(|text| text.terms.clone()).unwrap_or_default(),
                starts_at: row.starts_at,
                ends_at: row.ends_at,
                currency: row.currency,
                items: items
                    .iter()
                    .filter(|item| item.promotion_id == row.id)
                    .map(|item| PublicFlashItem {
                        variant_id: item.variant_id,
                        sale_price_minor: item.sale_price_minor,
                        value_bp: item.value_bp,
                        quantity_left: item
                            .quantity_limit
                            .map(|limit| (limit - item.quantity_used).max(0)),
                        per_customer_quantity_limit: item.per_customer_quantity_limit,
                    })
                    .collect(),
            }
        }))
    }
}
