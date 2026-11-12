//! Bảng `newsletter_suppressions`: địa chỉ không bao giờ nhận mail chiến dịch.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "newsletter_suppressions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Email đã chuẩn hoá, unique.
    pub email: String,
    /// `hard_bounce`, `soft_bounce`, `complaint` hoặc `manual`.
    pub reason: String,
    /// Ghi chú của quản trị viên hay mã lỗi SMTP.
    pub note: Option<String>,
    pub created_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}
