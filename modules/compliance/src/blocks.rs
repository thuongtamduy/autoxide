//! Chân trang pháp lý cho trang dựng từ khối (bước 67, D-229): thông tin chủ sở hữu website và logo
//! đã thông báo / đăng ký với Bộ Công Thương — Luật Thương mại điện tử 122/2025 và NĐ 248/2026 (thay
//! NĐ 52/2013, NĐ 85/2021 từ 01/7/2026) (cần đối chiếu văn bản gốc / người có chuyên môn pháp lý). `content` TỰ GẮN khối này
//! dưới mọi trang công khai của website (không nằm trong bố cục nên không xoá được); dữ liệu như
//! `GET /public/owner` và `GET /public/badge` của website đang xem. Chưa có hồ sơ pháp lý → `None`.

use std::sync::Arc;

use autoxide_kernel::blocks::{BlockCategory, BlockData, BlockRef, BlockRequest, BlockType};
use autoxide_kernel::error::Error;
use serde_json::{Value, json};

use crate::http_moit::{badge, owner};
use crate::service::ComplianceService;

pub(crate) const LEGAL_FOOTER: BlockType = BlockType::new(
    "compliance.legal_footer",
    BlockCategory::Legal,
    "Legal footer",
    "Never place this block: the website adds the owner information and the ministry logo below every page by itself.",
    r#"{"type": "object", "additionalProperties": false, "properties": {}}"#,
)
.with_data();

pub(crate) fn all() -> Vec<BlockType> {
    vec![LEGAL_FOOTER]
}

/// Bộ trả dữ liệu chân trang pháp lý.
pub(crate) struct LegalFooter(pub(crate) Arc<ComplianceService>);

#[async_trait::async_trait]
impl BlockData for LegalFooter {
    async fn resolve(
        &self,
        request: &BlockRequest<'_>,
        _block: &str,
        items: &[BlockRef],
    ) -> Result<Vec<Option<Value>>, Error> {
        let (tenant, org) = (request.tenant, request.site.org());
        let footer = match owner(&self.0, tenant, org).await? {
            Some(owner) => {
                // Website chưa thông báo / đăng ký (hay không phải website bán hàng): không logo.
                let badge = badge(&self.0, tenant, &[], org, request.locale).await.ok();
                Some(json!({
                    "owner": owner.data,
                    "badge": badge.map(|badge| badge.data),
                }))
            }
            None => None,
        };
        Ok(items.iter().map(|_| footer.clone()).collect())
    }
}
