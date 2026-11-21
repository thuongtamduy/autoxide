//! Bảng `newsletter_suppressions`: địa chỉ không bao giờ nhận mail chiến dịch.

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "newsletter_suppressions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Khách sở hữu dòng (bước 33).
    pub tenant_id: Uuid,
    /// Email đã chuẩn hoá, unique.
    pub email: String,
    /// `hard_bounce`, `soft_bounce`, `complaint` hoặc `manual`.
    pub reason: String,
    /// Ghi chú của quản trị viên hay mã lỗi SMTP.
    pub note: Option<String>,
    pub created_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
