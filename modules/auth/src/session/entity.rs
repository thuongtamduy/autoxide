//! Entity của bảng `auth_refresh_token` (SeaORM).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "auth_refresh_token")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub family_id: Uuid,
    pub user_id: Uuid,
    #[sea_orm(unique)]
    pub token_hash: String,
    pub expires_at: DateTimeUtc,
    pub used_at: Option<DateTimeUtc>,
    pub revoked_at: Option<DateTimeUtc>,
    pub created_at: DateTimeUtc,
    /// Header `User-Agent` lúc đăng nhập (cắt ở 255 ký tự); `None` với phiên cũ.
    pub user_agent: Option<String>,
    /// Lúc phiên (họ token) bắt đầu; `None` với phiên tạo trước m0015.
    pub session_started_at: Option<DateTimeUtc>,
}

impl ActiveModelBehavior for ActiveModel {}
