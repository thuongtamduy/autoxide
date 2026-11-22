//! Khối khuyến mại cho trang dựng từ khối (bước 67, bước 43): flash sale (đếm ngược, giá và suất
//! còn thật) và ô lưu voucher. Dữ liệu như đọc công khai: chỉ chương trình `public` của website
//! đang xem; chương trình đã hết hay không còn công khai → `None` (giao diện ẩn khối). Mỗi loại
//! khối một lần đọc cho mọi khối của trang.

use std::sync::Arc;

use autoxide_kernel::blocks::{BlockCategory, BlockData, BlockRef, BlockRequest, BlockType};
use autoxide_kernel::error::Error;
use serde_json::Value;
use uuid::Uuid;

use crate::service::PromotionsService;

pub(crate) const FLASH_SALE: BlockType = BlockType::new(
    "promotions.flash_sale",
    BlockCategory::Commerce,
    "Flash sale",
    "Use to show a flash sale with its countdown, sale prices and units left; never write prices or dates into text.",
    r#"{
        "type": "object",
        "additionalProperties": false,
        "required": ["promotion_id"],
        "properties": {
            "promotion_id": {"type": "string", "format": "uuid", "description": "A flash sale marked public."},
            "title": {"type": "string", "maxLength": 200}
        }
    }"#,
)
.with_data();

pub(crate) const VOUCHER_CLAIM: BlockType = BlockType::new(
    "promotions.voucher_claim",
    BlockCategory::Commerce,
    "Save a voucher",
    "Use to let buyers save a voucher of a running public promotion to use at checkout.",
    r#"{
        "type": "object",
        "additionalProperties": false,
        "required": ["promotion_id"],
        "properties": {
            "promotion_id": {"type": "string", "format": "uuid", "description": "A public promotion whose vouchers buyers can save (`claimable`)."},
            "button_label": {"type": "string", "maxLength": 60}
        }
    }"#,
)
.with_data();

pub(crate) fn all() -> Vec<autoxide_kernel::blocks::BlockType> {
    vec![FLASH_SALE, VOUCHER_CLAIM]
}

/// Bộ trả dữ liệu cho hai khối.
pub(crate) struct PromotionBlocks(pub(crate) Arc<PromotionsService>);

fn promotion_of(item: &BlockRef) -> Option<Uuid> {
    item.props.get("promotion_id").and_then(Value::as_str).and_then(|id| Uuid::parse_str(id).ok())
}

#[async_trait::async_trait]
impl BlockData for PromotionBlocks {
    async fn resolve(
        &self,
        request: &BlockRequest<'_>,
        block: &str,
        items: &[BlockRef],
    ) -> Result<Vec<Option<Value>>, Error> {
        let ids: Vec<Uuid> = items.iter().filter_map(promotion_of).collect();
        let (tenant, site, locale) = (request.tenant, request.site, request.locale);
        let found: Vec<(Uuid, Value)> = match block {
            "promotions.flash_sale" => self
                .0
                .public_flash_sales_by_id(tenant, site, locale, &ids)
                .await?
                .into_iter()
                .map(|sale| (sale.id, serde_json::to_value(&sale).unwrap_or(Value::Null)))
                .collect(),
            "promotions.voucher_claim" => self
                .0
                .public_promotions_by_id(tenant, site, locale, &ids)
                .await?
                .into_iter()
                // Chỉ chương trình cho lưu voucher; còn lại khối ẩn.
                .filter(|promotion| promotion.claimable)
                .map(|promotion| {
                    (promotion.id, serde_json::to_value(&promotion).unwrap_or(Value::Null))
                })
                .collect(),
            _ => Vec::new(),
        };
        Ok(items
            .iter()
            .map(|item| {
                let id = promotion_of(item)?;
                found.iter().find(|(found, _)| *found == id).map(|(_, value)| value.clone())
            })
            .collect())
    }
}
