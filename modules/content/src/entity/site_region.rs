//! Bảng `content_site_regions`: đầu trang / chân trang của website (bước 67, D-234).

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_site_regions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Khách sở hữu dòng (bước 33).
    pub tenant_id: Uuid,
    /// Website (nút của cây tổ chức).
    pub org_id: Uuid,
    /// `header` hay `footer`.
    pub region: String,
    pub locale: String,
    /// `base` hay ID bản chủ đề.
    pub variant_slot: String,
    /// Bố cục đã đăng (công khai).
    pub layout: Option<String>,
    /// Bản nháp đang sửa.
    pub draft_layout: Option<String>,
    pub published_at: Option<Timestamp>,
    pub updated_by: Option<Uuid>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
