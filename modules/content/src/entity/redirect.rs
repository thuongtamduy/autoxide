//! Bảng `content_redirects`: chuyển hướng từ một đường dẫn cũ.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_redirects")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Khách sở hữu dòng (bước 33).
    pub tenant_id: Uuid,
    pub locale: String,
    pub from_path: String,
    /// Đích là một mục (theo ID: không thành chuỗi chuyển hướng)…
    pub to_entry_id: Option<Uuid>,
    /// …hoặc một địa chỉ.
    pub to_url: Option<String>,
    /// 301, 302, 307, 308, hoặc 410 (đã gỡ).
    pub status_code: i32,
    /// `auto`, `manual` hoặc `gone`.
    pub source: String,
    pub created_by: Option<Uuid>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
