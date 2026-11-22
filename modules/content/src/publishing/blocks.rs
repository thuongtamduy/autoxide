//! Dữ liệu khối khi đọc công khai trang dựng từ khối (bước 67, D-20261122-layout-public): bỏ khối
//! khách không còn dùng được, rồi gọi `BlockData` của kernel — mỗi loại khối một lần, song song,
//! khối lỗi hay hết giờ là `null` (trang vẫn trả).

use std::collections::BTreeMap;

use autoxide_kernel::blocks::{BlockRef, BlockRequest};
use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{OrgId, Site, TenantContext};
use serde_json::Value;
use uuid::Uuid;

use crate::entries::layout;
use crate::service::ContentService;

/// Bố cục công khai và dữ liệu của khối cần dữ liệu (theo `id` của khối).
pub(crate) struct PublicLayout {
    pub body: Value,
    pub data: BTreeMap<String, Value>,
}

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
        let refs: Vec<(&str, BlockRef)> = layout::blocks_of(&body)
            .into_iter()
            .filter(|(name, _, _)| usable.get(name).is_some_and(|info| info.block.has_data))
            .map(|(name, id, props)| (name, BlockRef::new(id, props.clone())))
            .collect();
        let request = BlockRequest::new(tenant, site, locale);
        let data = self.blocks.resolve(&request, &refs).await;
        Ok(Some(PublicLayout { body, data }))
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
