//! Bảng `forms_form_versions`: ảnh chụp danh sách trường mỗi khi trường đổi — bài gửi cũ luôn
//! đọc được với đúng nhãn lúc gửi.

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "forms_form_versions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Khách sở hữu dòng (bước 33).
    pub tenant_id: Uuid,
    pub form_id: Uuid,
    pub version: i32,
    /// JSON: danh sách trường của phiên bản này.
    pub fields: String,
    pub created_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
