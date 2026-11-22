//! Bảng `content_page_templates`: trang mẫu của khách (bước 67).

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_page_templates")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Khách sở hữu dòng (bước 33).
    pub tenant_id: Uuid,
    pub org_id: Uuid,
    pub industry: String,
    pub locale: String,
    pub name: String,
    pub description: String,
    /// Bố cục khối JSON.
    pub layout: String,
    pub preset_key: Option<String>,
    pub created_by: Option<Uuid>,
    pub created_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
