//! Ước tính phí giao công khai cho trang sản phẩm (bước 44 mục 7 và 9, D-200): một giỏ ảo một dòng
//! giao tới một tỉnh (chưa cần xã), chạy đúng hàm báo giá mà `orders` dùng; kết quả ghi rõ
//! `estimate: true` — phí thật luôn tính lại khi đặt hàng. Số liệu của biến thể (cân, kích thước,
//! nút bán) và giá (để xét miễn phí) lấy qua `autoxide_catalog::api::Catalog`.

use autoxide_catalog::api::{PriceQuery, ProductStatus};
use autoxide_kernel::error::Error;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::tenancy::{OrgId, TenantContext};
use autoxide_kernel::time::Timestamp;
use uuid::Uuid;

use crate::addresses::address_of;
use crate::api::{Quote, QuoteItem, QuoteRequest};
use crate::errors;
use crate::service::ShippingService;

/// Câu hỏi ước tính.
#[derive(Debug, Clone)]
pub(crate) struct EstimateQuery {
    pub variant_id: Uuid,
    pub product_id: Option<Uuid>,
    pub quantity: i64,
    pub country: String,
    pub province_code: String,
    /// Cửa hàng bán (website của chuỗi bán ở một cửa hàng); mặc định: nút sở hữu sản phẩm.
    pub org_id: Option<Uuid>,
    pub currency: Currency,
    pub locale: String,
}

impl ShippingService {
    /// Ước tính phí và ngày giao tới tỉnh cho `quantity` món của một biến thể đang bán.
    pub(crate) async fn estimate(
        &self,
        tenant: &TenantContext,
        query: EstimateQuery,
    ) -> Result<Quote, Error> {
        let catalog = self.catalog.as_ref().ok_or_else(errors::estimate_unavailable)?;
        if !(1..=1000).contains(&query.quantity) {
            return Err(errors::estimate_quantity());
        }
        let variant = catalog
            .variants(tenant, &[query.variant_id], Some(&query.locale))
            .await?
            .into_iter()
            .next()
            .filter(|variant| variant.status == ProductStatus::Active && variant.variant_active)
            .filter(|variant| query.product_id.is_none_or(|product| product == variant.product_id))
            .ok_or_else(|| errors::variant_not_for_sale(query.variant_id))?;
        let org = query.org_id.map_or(variant.org_id, OrgId::from_uuid);
        let at = Timestamp::now();
        let price = catalog
            .prices(tenant, &PriceQuery::new(org, query.currency, at, vec![variant.variant_id]))
            .await?
            .into_iter()
            .next()
            .map_or(Money::zero(query.currency), |price| price.amount);
        let value = price.checked_mul(query.quantity)?;
        let item = QuoteItem::new(variant.variant_id, query.quantity)
            .sku(variant.sku.clone())
            .requires_shipping(variant.requires_shipping)
            .weight(variant.weight_grams)
            .dimensions(variant.length_mm, variant.width_mm, variant.height_mm);
        let country = query.country.trim().to_ascii_uppercase();
        let province = query.province_code.trim();
        // Chưa có xã: chỉ tỉnh. Không kiểm danh mục hành chính (vùng phí chỉ cần tỉnh).
        let destination = address_of(&country, province, province, "", "", "")?;
        let request = QuoteRequest::new(org, vec![item], value, at)
            .destination(destination)
            .locale(query.locale);
        self.quote_unchecked(tenant, &request).await
    }
}
