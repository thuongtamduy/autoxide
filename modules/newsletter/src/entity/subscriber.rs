//! Bảng `newsletter_subscribers`: một dòng cho mỗi địa chỉ email, dùng chung cho mọi danh sách.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "newsletter_subscribers")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Khách sở hữu dòng (bước 33).
    pub tenant_id: Uuid,
    /// Email đã chuẩn hoá (trim + chữ thường), unique.
    pub email: String,
    pub name: Option<String>,
    /// Ngôn ngữ của mail xác nhận và chân thư: `en` hoặc `vi`.
    pub locale: String,
    /// `active` hoặc `blocked`.
    pub status: String,
    pub blocked_reason: Option<String>,
    /// JSON object do quản trị viên đặt.
    pub attributes: String,
    /// Số lần soft bounce từ lần bị chặn gần nhất.
    pub soft_bounces: i32,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
