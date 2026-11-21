//! Đồng bộ nhân viên qua SCIM 2.0 (bước 66, RFC 7643 / RFC 7644, D-20261121-scim).
//!
//! Hệ thống danh tính của khách (Microsoft Entra ID, Okta, Google Workspace) gọi endpoint SCIM
//! dưới `/api/v1/auth/scim/v2` bằng **token SCIM** của khách (`axscim1_<ID khách>_<ngẫu nhiên>`,
//! bảng `auth_scim_tokens`, chỉ lưu bản băm). Token chỉ dùng được ở endpoint SCIM: bộ xác thực
//! chung không nhận nó (không phải API key, không phải access token), và API key / access token
//! không dùng được ở đây.
//!
//! - `Group` ↔ nhóm của hệ thống danh tính; luật ánh xạ của khách (nhóm X → role R ở nút N) biến
//!   thành viên thành membership ([`groups`], [`mappings`]).
//! - `User` ↔ tài khoản nhân sự (`realm = staff`) của khách; liên kết ở `auth_scim_users`
//!   (`userName`, `externalId`, tên riêng / họ). Email và tên hiển thị nằm ở `auth_user`.
//! - `active = false` → trong CÙNG transaction: khoá tài khoản, thu hồi mọi phiên / refresh
//!   token và API key, ghi event (`actor_kind = scim`).
//! - `DELETE` → quy trình xoá tài khoản sẵn có (`UserService::delete`).
//!
//! Lỗi theo lược đồ lỗi SCIM ([`error`]), danh sách theo `ListResponse`, phân trang
//! `startIndex` / `count` — ngoại lệ có chủ đích với envelope chung của Autoxide.

pub(crate) mod discovery;
pub(crate) mod entity;
pub(crate) mod error;
pub(crate) mod filter;
mod groups;
#[cfg(test)]
mod groups_tests;
mod mappings;
pub(crate) mod patch;
#[cfg(test)]
mod tests;
mod token;
mod users;

use std::sync::Arc;

use autoxide_kernel::access::ActorKind;
use autoxide_kernel::entitlement::{EntitlementsHandle, Feature};
use autoxide_kernel::event::{Actor, Outbox};
use autoxide_kernel::tenancy::{DatabaseRegistry, ScopeExt as _, TenantContext};
use sea_orm::{ColumnTrait as _, ConnectionTrait, EntityTrait as _, QueryFilter as _};
use uuid::Uuid;

pub(crate) use self::mappings::{GroupMapping, GroupSummary, UnmappedGroups};
pub(crate) use self::token::{CreatedScimToken, ScimToken};
use crate::user::UserService;

/// Đường dẫn gốc của endpoint SCIM (cho `meta.location`, tài liệu).
pub(crate) const BASE_PATH: &str = "/api/v1/auth/scim/v2";
pub(crate) const USER_SCHEMA: &str = "urn:ietf:params:scim:schemas:core:2.0:User";
pub(crate) const GROUP_SCHEMA: &str = "urn:ietf:params:scim:schemas:core:2.0:Group";
pub(crate) const LIST_SCHEMA: &str = "urn:ietf:params:scim:api:messages:2.0:ListResponse";

/// Tính năng theo gói (bước 35): đồng bộ nhân viên qua SCIM.
pub(crate) const SCIM_FEATURE: Feature = Feature::new(
    "auth.scim",
    "Sync staff accounts and groups from the company's identity provider (Microsoft Entra ID, \
     Okta, Google Workspace) over SCIM 2.0.",
);

/// Service SCIM. Không giữ database: mỗi hàm nhận khách (của token, hay của request quản trị).
pub(crate) struct ScimService {
    /// Sổ database: tìm khách ghi trong token.
    pub databases: DatabaseRegistry,
    pub outbox: Outbox,
    pub entitlements: EntitlementsHandle,
    pub users: Arc<UserService>,
    /// Đồng bộ membership do nhóm SCIM cấp.
    pub roles: Arc<crate::role::RoleService>,
}

/// Bên gọi endpoint SCIM đã xác thực: khách của token.
pub(crate) struct ScimCaller {
    pub tenant: TenantContext,
    /// Người tạo token: SCIM chỉ nhận về, khoá hay đổi email tài khoản mà người này bao trùm quyền
    /// (như D-079 với quản trị viên).
    pub created_by: Uuid,
}

impl ScimCaller {
    /// Người làm ghi vào event / audit: `actor_kind = scim`.
    pub(crate) fn actor(&self) -> Actor {
        let mut actor = Actor::system();
        actor.kind = Some(ActorKind::Scim);
        actor
    }
}

/// Xoá liên kết SCIM, thành viên nhóm và dấu "do SCIM cấp" của tài khoản `user_id` trong
/// transaction xoá tài khoản (không có foreign key, như `auth_identity`).
pub(crate) async fn forget_user<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    user_id: Uuid,
) -> Result<(), autoxide_kernel::error::Error> {
    entity::scim_user::Entity::delete_many()
        .filter(entity::scim_user::Column::Id.eq(user_id))
        .scoped(tenant)
        .exec(connection)
        .await?;
    entity::member::Entity::delete_many()
        .filter(entity::member::Column::UserId.eq(user_id))
        .scoped(tenant)
        .exec(connection)
        .await?;
    crate::role::forget_scim_grants(connection, tenant, user_id).await
}

/// Liên kết SCIM của tài khoản `user_id` (bản xuất dữ liệu cá nhân); `None`: không có.
pub(crate) async fn link_of(
    tenant: &TenantContext,
    user_id: Uuid,
) -> Result<Option<entity::scim_user::Model>, autoxide_kernel::error::Error> {
    Ok(entity::scim_user::Entity::find_by_id((tenant.id().as_uuid(), user_id))
        .one(tenant.database().read())
        .await?)
}
