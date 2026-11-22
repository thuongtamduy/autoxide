//! Dữ liệu khối khi đọc công khai trang dựng từ khối (bước 67, D-20261122-layout-public): bỏ khối
//! khách không còn dùng được, rồi gọi `BlockData` của kernel — mỗi loại khối một lần, song song,
//! khối lỗi hay hết giờ là `null` (trang vẫn trả).

use std::collections::BTreeMap;

use autoxide_kernel::blocks::{BlockRef, BlockRequest};
use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{OrgId, Site, TenantContext};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::entries::layout;
use crate::service::ContentService;

/// Bố cục công khai và dữ liệu của khối cần dữ liệu (theo `id` của khối).
pub(crate) struct PublicLayout {
    pub body: Value,
    pub data: BTreeMap<String, Value>,
    /// Chân trang pháp lý tự gắn (khối `compliance.legal_footer`), khi khách có module đó.
    pub legal_footer: Option<Value>,
}

/// Khối chân trang pháp lý của `compliance`: tự gắn dưới mọi trang, không đặt trong bố cục.
pub(crate) const LEGAL_FOOTER: &str = "compliance.legal_footer";
/// `id` giả của chân trang pháp lý trong lần đọc dữ liệu khối: không phải `id` hợp lệ của khối
/// (`$`), nên không trùng khối nào của trang.
const LEGAL_FOOTER_ID: &str = "$legal_footer";

impl ContentService {
    /// Khối khách `tenant` đang dùng được, theo tên.
    pub(crate) async fn usable_blocks(
        &self,
        tenant: &TenantContext,
    ) -> Result<layout::Usable, Error> {
        Ok(self
            .blocks
            .usable(tenant)
            .await?
            .into_iter()
            .map(|info| (info.block.name, info))
            .collect())
    }

    /// Thân đã lưu `body` là bố cục khối thì trả bản công khai kèm dữ liệu khối; thân `rich` thì
    /// `None`. `site`: website đang xem (bước 34), `locale`: ngôn ngữ của mục.
    pub(crate) async fn public_layout(
        &self,
        tenant: &TenantContext,
        site: &Site,
        locale: &str,
        body: &str,
    ) -> Result<Option<PublicLayout>, Error> {
        // Đường nóng của thân `rich`: không đọc JSON khi chắc chắn không phải bố cục.
        if !body.contains("\"layout\"") {
            return Ok(None);
        }
        let Ok(parsed) = serde_json::from_str::<Value>(body) else {
            return Ok(None);
        };
        if !layout::is_layout(&parsed) {
            return Ok(None);
        }
        let usable = self.usable_blocks(tenant).await?;
        let body = layout::public(&parsed, &usable);
        let mut refs: Vec<(&str, BlockRef)> = layout::blocks_of(&body)
            .into_iter()
            .filter(|(name, _, _)| usable.get(name).is_some_and(|info| info.block.has_data))
            .map(|(name, id, props)| (name, BlockRef::new(id, props.clone())))
            .collect();
        let footer = usable.contains_key(LEGAL_FOOTER);
        if footer {
            refs.push((LEGAL_FOOTER, BlockRef::new(LEGAL_FOOTER_ID, Value::Object(Map::new()))));
        }
        let request = BlockRequest::new(tenant, site, locale);
        let mut data = self.blocks.resolve(&request, &refs).await;
        let legal_footer = data.remove(LEGAL_FOOTER_ID).filter(|value| footer && !value.is_null());
        Ok(Some(PublicLayout { body, data, legal_footer }))
    }

    /// Website của mục có nút `org` (bước 34), cả khách khi không có hay nút không còn.
    pub(crate) async fn site_of(&self, tenant: &TenantContext, org: Option<Uuid>) -> Site {
        match org {
            Some(org) => Site::of(tenant, OrgId::from_uuid(org))
                .await
                .unwrap_or_else(|_| Site::whole(tenant)),
            None => Site::whole(tenant),
        }
    }

    /// ID `media` của thân đã lưu: ảnh (node `image`) của thân `rich`, props `x-media` của bố cục.
    pub(crate) fn body_media_ids(&self, body: &str) -> Vec<Uuid> {
        if body.contains("\"layout\"")
            && let Ok(parsed) = serde_json::from_str::<Value>(body)
            && layout::is_layout(&parsed)
        {
            return layout::media_ids(&parsed, &self.blocks);
        }
        crate::entries::body::stored_image_ids(body)
    }
}
