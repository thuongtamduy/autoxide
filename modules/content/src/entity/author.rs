//! Bảng `content_authors`: hồ sơ tác giả của `content`.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_authors")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Khách sở hữu dòng (bước 33).
    pub tenant_id: Uuid,
    /// Tài khoản gắn với hồ sơ (không foreign key sang `auth`); để trống được.
    pub user_id: Option<Uuid>,
    pub display_name: String,
    pub slug: String,
    pub bio: String,
    pub avatar_media_id: Option<Uuid>,
    pub website_url: String,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
