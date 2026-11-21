//! Bảng `content_banners`: banner của một vị trí (bước 55, D-236).

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_banners")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Khách sở hữu dòng (bước 33).
    pub tenant_id: Uuid,
    pub placement_id: Uuid,
    /// Nút sở hữu; gốc (= ID khách) nghĩa là mọi cửa hàng của khách.
    pub org_id: Uuid,
    pub locale: String,
    /// `image` hay `text`.
    pub kind: String,
    pub title: String,
    pub subtitle: String,
    pub cta_label: String,
    /// Banner chữ: token màu của theme.
    pub tone: Option<String>,
    pub media_id: Option<Uuid>,
    pub mobile_media_id: Option<Uuid>,
    pub alt: String,
    /// JSON mảng loại trang; rỗng = mọi loại của vị trí.
    pub page_kinds: String,
    /// JSON mảng `{kind, id}`; rỗng = mọi trang.
    pub page_targets: String,
    /// `none`, `url` hay một loại của sổ đối tượng.
    pub link_kind: String,
    pub link_url: Option<String>,
    pub link_id: Option<String>,
    pub starts_at: Option<Timestamp>,
    pub ends_at: Option<Timestamp>,
    pub position: i32,
    /// `draft`, `active`, `archived`.
    pub status: String,
    pub ai_assisted: bool,
    pub created_by: Option<Uuid>,
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
