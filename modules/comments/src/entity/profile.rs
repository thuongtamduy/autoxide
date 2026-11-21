//! Bảng `comments_profiles`: tên hiển thị người dùng tự đặt. `Principal` không có tên và module
//! không đọc `auth` (D-046), nên tên nằm ở đây; `user_id` không có foreign key. Khoá chính
//! (`tenant_id`, `user_id`).

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "comments_profiles")]
pub struct Model {
    /// Khách sở hữu dòng (bước 33), cột đầu của khoá chính.
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub user_id: Uuid,
    pub display_name: String,
    /// `text::name_key` của tên, unique trong khách: hai người không trùng tên, kể cả tên viết bằng chữ Kirin /
    /// Hy Lạp trông như tên Latin (chống mạo danh).
    pub name_key: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
