//! Uỷ quyền vào sandbox (bước 38, D-232): nhân viên của khách gốc vào sandbox bằng đường uỷ quyền của
//! bước 34 (header `Autoxide-Tenant`), không có tài khoản riêng trong sandbox.
//!
//! Nghe event của kernel ghi ở khách gốc:
//! - `kernel.sandbox_created` → một dòng `auth_delegation` (khách gốc → sandbox) cho role đã chọn
//!   (mặc định role `admin` của khách gốc), ở nút gốc của sandbox, hết hạn cùng sandbox;
//! - `kernel.sandbox_extended` → dời hạn của uỷ quyền đó;
//! - `kernel.sandbox_deleted` → thu hồi.
//!
//! Nhận lại event không tạo dòng thứ hai (D-054): tìm uỷ quyền còn hiệu lực cho cùng sandbox, cùng
//! role trước khi ghi. Không sửa `DelegationService::enter`: uỷ quyền vào sandbox là uỷ quyền thường.

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler, Subscription};
use autoxide_kernel::tenancy::{ScopeExt, TenantId, root_of};
use chrono::{DateTime, SubsecRound, Utc};
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

fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
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
        let expires_at: Option<DateTime<Utc>> =
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
                let chosen = match uuid(event, "role_id") {
                    Some(role)
                        if crate::role::role_grants(source, &[role]).await?.contains_key(&role) =>
                    {
                        Some(role)
                    }
                    _ => crate::role::role_by_name(source, "admin").await?,
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
