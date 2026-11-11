//! Entity của bảng `auth_user` (SeaORM). Chỉ dùng trong module này; ra ngoài thì
//! đổi sang [`crate::api::User`], không bao giờ mang theo hash mật khẩu.

use sea_orm::entity::prelude::*;

use crate::api::User;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "auth_user")]
pub struct Model {
    // UUIDv7 sinh trong ứng dụng, không để database tự tăng.
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    #[sea_orm(unique)]
    pub email: String,
    pub name: String,
    pub password_hash: Option<String>,
    pub disabled_at: Option<DateTimeUtc>,
    pub email_verified_at: Option<DateTimeUtc>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl From<Model> for User {
    fn from(model: Model) -> Self {
        User {
            id: model.id,
            email: model.email,
            name: model.name,
            disabled_at: model.disabled_at,
            email_verified_at: model.email_verified_at,
            created_at: model.created_at,
            updated_at: model.updated_at,
        }
    }
}
