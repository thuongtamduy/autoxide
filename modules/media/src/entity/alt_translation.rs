//! Bảng `media_alt_translations`: `alt` của một item bằng một ngôn ngữ khác ngôn ngữ mặc định
//! (D-202). `media_item.alt` vẫn là bản mặc định, dùng khi ngôn ngữ được hỏi chưa có bản dịch.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "media_alt_translations")]
pub struct Model {
    /// Khách sở hữu dòng (bước 33), cột đầu của khoá chính.
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub media_id: Uuid,
    /// Mã BCP 47, đúng như khai trong `AUTOXIDE_MEDIA_ALT_LOCALES`.
    #[sea_orm(primary_key, auto_increment = false)]
    pub locale: String,
    pub alt: String,
    pub updated_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
