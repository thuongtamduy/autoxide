//! Bảng `content_types`: loại nội dung tự định nghĩa của khách (bước 67, D-235).

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_types")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Khách sở hữu dòng (bước 33).
    pub tenant_id: Uuid,
    /// Website của loại (nút; gốc = cả khách).
    pub org_id: Uuid,
    /// `snake_case`, không đổi khi loại đã có mục.
    pub key: String,
    /// JSON: `{"one": {lang: nhãn}, "many": {lang: nhãn}}`.
    pub labels: String,
    pub icon: String,
    pub routable: bool,
    pub path_prefix: Option<String>,
    pub body: String,
    pub has_seo: bool,
    pub listable: bool,
    pub active: bool,
    pub created_by: Option<Uuid>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
