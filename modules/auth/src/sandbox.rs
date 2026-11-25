//! Uỷ quyền vào sandbox (bước 38, D-232): nhân viên của khách gốc vào sandbox bằng đường uỷ quyền của
//! bước 34 (header `Autoxide-Tenant`), không có tài khoản riêng trong sandbox.
//!
//! Nghe event của kernel ghi ở khách gốc:
//! - `kernel.sandbox_created` → một dòng `auth_delegation` (khách gốc → sandbox) cho role đã chọn
//!   (mặc định role `admin` của khách gốc), ở nút gốc của sandbox, hết hạn cùng sandbox;
//! - `kernel.sandbox_extended` → dời hạn của uỷ quyền đó;
//! - `kernel.sandbox_deleted` → thu hồi.
//!
//! Công bố `SandboxRecipients` của kernel: mail của sandbox chỉ tới email đã xác minh của nhân viên
//! khách gốc đang giữ role được uỷ quyền vào sandbox (uỷ quyền còn hiệu lực); người nhận khác vào
//! hộp thư giả (D-232).
//!
//! Nhận lại event không tạo dòng thứ hai (D-054): tìm uỷ quyền còn hiệu lực cho cùng sandbox, cùng
//! role trước khi ghi. Không sửa `DelegationService::enter`: uỷ quyền vào sandbox là uỷ quyền thường.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Delivered, EventHandler, Subscription};
use autoxide_kernel::pagination::PageRequest;
use autoxide_kernel::tenancy::sandbox::SandboxRecipients;
use autoxide_kernel::tenancy::{DatabaseRegistry, ScopeExt, TenantContext, TenantId, root_of};
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use uuid::Uuid;

use crate::delegation::entity;

pub(crate) const SANDBOX_CREATED: &str = "kernel.sandbox_created";
pub(crate) const SANDBOX_EXTENDED: &str = "kernel.sandbox_extended";
pub(crate) const SANDBOX_DELETED: &str = "kernel.sandbox_deleted";

/// Lý do ghi trên uỷ quyền (vào audit của khách gốc).
const REASON: &str = "Sandbox access for the staff of the tenant";

/// Bộ nghe ba event sandbox của kernel.
pub(crate) struct SandboxAccess;

pub(crate) fn subscriptions() -> Vec<Subscription> {
    let handler = Arc::new(SandboxAccess);
    vec![
        Subscription::new(SANDBOX_CREATED, handler.clone()),
        Subscription::new(SANDBOX_EXTENDED, handler.clone()),
        Subscription::new(SANDBOX_DELETED, handler),
    ]
}

fn now() -> Timestamp {
    Timestamp::now()
}

fn uuid(event: &Delivered, field: &str) -> Option<Uuid> {
    event.payload[field].as_str().and_then(|raw| raw.parse().ok())
}

#[async_trait::async_trait]
impl EventHandler for SandboxAccess {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        // Event ghi ở khách gốc: `event.tenant` là khách cấp uỷ quyền.
        let source = &event.tenant;
        let Some(sandbox) = uuid(event, "sandbox_id") else {
            return Err(Error::internal("a sandbox event without `sandbox_id`"));
        };
        let expires_at: Option<Timestamp> =
            serde_json::from_value(event.payload["expires_at"].clone()).unwrap_or(None);
        let live = || {
            entity::Entity::find()
                .filter(entity::Column::ChildTenantId.eq(sandbox))
                .filter(entity::Column::RevokedAt.is_null())
                .scoped(source)
        };
        let write = source.database().write();
        match event.name.as_str() {
            SANDBOX_CREATED => {
                // Vai trò được chọn mà không có ở khách (ID bịa hay của khách khác): không uỷ quyền gì,
                // không rơi về `admin` (PT-8). Không chọn vai trò thì mặc định `admin` như trước.
                let chosen = match uuid(event, "role_id") {
                    Some(role)
                        if crate::role::role_grants(source, &[role]).await?.contains_key(&role) =>
                    {
                        Some(role)
                    }
                    Some(_) => {
                        tracing::warn!(sandbox_id = %sandbox, "sandbox role not found; no delegation");
                        None
                    }
                    None => crate::role::role_by_name(source, "admin").await?,
                };
                let Some(role) = chosen else {
                    tracing::warn!(sandbox_id = %sandbox, "no role to delegate into the sandbox");
                    return Ok(());
                };
                if live().filter(entity::Column::RoleId.eq(role)).one(write).await?.is_some() {
                    return Ok(());
                }
                entity::ActiveModel {
                    tenant_id: Set(source.id().as_uuid()),
                    id: Set(Uuid::now_v7()),
                    child_tenant_id: Set(sandbox),
                    role_id: Set(role),
                    org_id: Set(root_of(TenantId::from_uuid(sandbox)).as_uuid()),
                    reason: Set(REASON.to_owned()),
                    granted_by: Set(event.actor.user_id),
                    expires_at: Set(expires_at),
                    revoked_at: Set(None),
                    revoked_by: Set(None),
                    created_at: Set(now()),
                }
                .insert(write)
                .await?;
                tracing::info!(sandbox_id = %sandbox, "sandbox delegation granted");
            }
            SANDBOX_EXTENDED => {
                entity::Entity::update_many()
                    .col_expr(entity::Column::ExpiresAt, Expr::value(expires_at))
                    .filter(entity::Column::ChildTenantId.eq(sandbox))
                    .filter(entity::Column::RevokedAt.is_null())
                    .scoped(source)
                    .exec(write)
                    .await?;
            }
            SANDBOX_DELETED => {
                entity::Entity::update_many()
                    .col_expr(entity::Column::RevokedAt, Expr::value(Some(now())))
                    .filter(entity::Column::ChildTenantId.eq(sandbox))
                    .filter(entity::Column::RevokedAt.is_null())
                    .scoped(source)
                    .exec(write)
                    .await?;
            }
            _ => {}
        }
        Ok(())
    }
}

/// Ai trong khách gốc nhận được mail thật của sandbox.
pub(crate) struct StaffRecipients {
    pub(crate) databases: DatabaseRegistry,
}

#[async_trait::async_trait]
impl SandboxRecipients for StaffRecipients {
    async fn allowed(&self, sandbox: &TenantContext, email: &str) -> Result<bool, Error> {
        let Some(source_id) = sandbox.sandbox_of() else { return Ok(false) };
        let source = self.databases.tenant(source_id).await?;
        let at = now();
        let roles: Vec<Uuid> = entity::Entity::find()
            .filter(entity::Column::ChildTenantId.eq(sandbox.id().as_uuid()))
            .filter(entity::Column::RevokedAt.is_null())
            .scoped(&source)
            .all(source.database().read())
            .await?
            .into_iter()
            .filter(|row| row.expires_at.is_none_or(|expires| expires > at))
            .map(|row| row.role_id)
            .collect();
        if roles.is_empty() {
            return Ok(false);
        }
        let user = crate::user::entity::Entity::find()
            .filter(crate::user::entity::Column::Realm.eq("staff"))
            .filter(crate::user::entity::Column::Email.eq(autoxide_kernel::email::normalize(email)))
            .filter(crate::user::entity::Column::EmailVerifiedAt.is_not_null())
            .filter(crate::user::entity::Column::DisabledAt.is_null())
            .scoped(&source)
            .one(source.database().read())
            .await?;
        let Some(user) = user else { return Ok(false) };
        crate::role::holds_any_role(&source, user.id, &roles).await
    }
}

/// Sao role của khách gốc vào sandbox (D-232): tên, mô tả, permission, độ sâu, cấm, MFA. Không sao
/// người dùng, membership hay API key. Role `admin` đã được gieo khi tạo khách nên bỏ qua; role trùng
/// tên đã có ở sandbox (chạy lại) cũng bỏ qua.
pub(crate) struct RoleCopy {
    pub(crate) roles: Arc<crate::role::RoleService>,
}

/// Loại bản ghi của bảng nối sandbox.
const ROLE_KIND: &str = "auth.role";

#[async_trait::async_trait]
impl autoxide_kernel::tenancy::sandbox::SandboxCopy for RoleCopy {
    async fn copy(
        &self,
        context: &autoxide_kernel::tenancy::sandbox::CopyContext<'_>,
    ) -> Result<u64, Error> {
        use autoxide_kernel::tenancy::sandbox::{copied, link_copy};

        let mut count = 0;
        let first = PageRequest::first(200, autoxide_kernel::pagination::Order::Asc);
        let mut cursor = first.clone();
        loop {
            let page = self.roles.list(context.source, &cursor).await?;
            for role in &page.data {
                if role.all_permissions
                    || copied(context.sandbox, ROLE_KIND, role.id).await?.is_some()
                {
                    continue;
                }
                if crate::role::role_by_name(context.sandbox, &role.name).await?.is_some() {
                    continue;
                }
                let input = crate::role::RoleInput {
                    name: role.name.clone(),
                    description: role.description.clone(),
                    permissions: Vec::new(),
                    grants: role.grants.clone(),
                    requires_mfa: role.requires_mfa,
                    denies: role.denies.clone(),
                };
                let made = self.roles.create(context.sandbox, Actor::system(), input).await?;
                link_copy(context.sandbox, ROLE_KIND, made.id, role.id).await?;
                count += 1;
            }
            match page.data.last() {
                Some(last) if page.data.len() == 200 => cursor = first.clone().after(last.id),
                _ => break,
            }
        }
        Ok(count)
    }
}
