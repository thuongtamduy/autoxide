//! Bảng `content_placements`: vị trí hiển thị banner theo khoá (bước 55, D-236).

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_placements")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Khách sở hữu dòng (bước 33).
    pub tenant_id: Uuid,
    pub key: String,
    pub name: String,
    pub description: String,
    pub max_items: i32,
    /// `site` (hay `seller_store`, D-233).
    pub audience: String,
    /// `single`, `carousel`, `grid`, `stack`, `strip`, `popup`.
    pub display: String,
    /// JSON object, theo `display`.
    pub display_settings: String,
    /// `image`, `text`, `any`.
    pub accepts: String,
    pub ratio_w: Option<i32>,
    pub ratio_h: Option<i32>,
    pub mobile_ratio_w: Option<i32>,
    pub mobile_ratio_h: Option<i32>,
    pub min_width: Option<i32>,
    pub min_height: Option<i32>,
    /// `all`, `desktop`, `mobile`.
    pub devices: String,
    /// JSON mảng loại trang.
    pub page_kinds: String,
    pub preset_key: Option<String>,
    pub active: bool,
    pub created_at: Timestamp,
    /// Phiên bản (`ETag`).
    pub updated_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
