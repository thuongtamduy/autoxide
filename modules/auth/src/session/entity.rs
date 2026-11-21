//! Entity của bảng `auth_refresh_token` (SeaORM).

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "auth_refresh_token")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Khách sở hữu dòng (bước 33).
    pub tenant_id: Uuid,
    pub family_id: Uuid,
    pub user_id: Uuid,
    pub token_hash: String,
    pub expires_at: Timestamp,
    pub used_at: Option<Timestamp>,
    pub revoked_at: Option<Timestamp>,
    pub created_at: Timestamp,
    /// Header `User-Agent` lúc đăng nhập (cắt ở 255 ký tự); `None` với phiên cũ.
    pub user_agent: Option<String>,
    /// Lúc phiên (họ token) bắt đầu; `None` với phiên tạo trước m0015.
    pub session_started_at: Option<Timestamp>,
    /// Cách mở phiên (`password`, `password_and_mfa`, `passkey`, `password_change`); `None` với
    /// phiên tạo trước m0024.
    pub sign_in_method: Option<String>,
    /// Quốc gia gần đúng (ISO 3166-1 hai chữ) từ header của proxy tin cậy, khi có (D-272).
    pub country: Option<String>,
    /// Lần đăng nhập hay xác nhận lại gần nhất của phiên (bước 54 P5); `None`: lúc phiên bắt đầu.
    pub authenticated_at: Option<Timestamp>,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
