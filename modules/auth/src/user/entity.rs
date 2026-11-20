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
    /// Khách sở hữu dòng (bước 33).
    pub tenant_id: Uuid,
    pub email: String,
    pub name: String,
    pub password_hash: Option<String>,
    pub disabled_at: Option<DateTimeUtc>,
    pub email_verified_at: Option<DateTimeUtc>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    /// Lúc mở phiên gần nhất (đăng nhập, đổi mật khẩu); `None`: chưa biết.
    pub last_sign_in_at: Option<DateTimeUtc>,
    /// Email và tên viết thường, để tìm (`user::service::search_text`).
    pub search_text: String,
    /// Ngôn ngữ ưa thích (mã BCP 47 máy chủ có bản dịch); `None`: theo request.
    pub locale: Option<String>,
    /// Múi giờ IANA ưa thích; `None`: chưa đặt.
    pub time_zone: Option<String>,
    /// `staff` hay `customer` (D-309).
    pub realm: String,
    /// HMAC của số điện thoại dùng để đăng nhập (`SecretKey::phone_key`); `None`: không đăng nhập bằng
    /// số. Tài khoản chỉ có số thì `email` là `''` (bước 47).
    pub phone_key: Option<String>,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}

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
            realm: realm_of(&model.realm),
            locale: model.locale,
            time_zone: model.time_zone,
        }
    }
}

/// `realm` lưu trong database → kiểu của kernel; giá trị lạ coi như nhân sự (mặc định của cột).
pub(crate) fn realm_of(raw: &str) -> autoxide_kernel::access::Realm {
    if raw == "customer" {
        autoxide_kernel::access::Realm::Customer
    } else {
        autoxide_kernel::access::Realm::Staff
    }
}

/// Giá trị cột `realm` của `realm`.
pub(crate) fn realm_str(realm: autoxide_kernel::access::Realm) -> &'static str {
    match realm {
        autoxide_kernel::access::Realm::Customer => "customer",
        autoxide_kernel::access::Realm::Staff => "staff",
    }
}
