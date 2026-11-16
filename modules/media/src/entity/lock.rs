//! Bảng `media_lock`: mỗi khách một dòng `folder_tree`, cập nhật đầu mỗi lệnh chuyển thư mục để
//! các lệnh đó (của cùng khách) chạy lần lượt (D-086). Khách mặc định có dòng từ migration
//! `m0001`; khách khác có dòng khi lần đầu chuyển thư mục.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "media_lock")]
pub struct Model {
    /// Khách sở hữu dòng (bước 33), cột đầu của khoá chính.
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub name: String,
    pub counter: i64,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
