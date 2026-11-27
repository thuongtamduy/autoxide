//! Grant OAuth: liệt kê, thu hồi (người dùng thu hồi của mình, quản trị thu hồi của khách), và thu hồi theo vòng đời
//! tài khoản (khoá, xoá, lấy lại tài khoản, đổi mật khẩu kèm đăng xuất khắp nơi — D-20261126-mcp-oauth mục 9).

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Outbox};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{ScopeExt as _, TenantContext};
use autoxide_kernel::time::Timestamp;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait as _, ConnectionTrait, EntityTrait as _, QueryFilter as _};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use super::clients::host_of;
use super::entity::{grant, token};
use super::{OAuthService, require_person};
use crate::events;

/// Chủ đề event của một grant.
pub(crate) fn subject(id: Uuid) -> Option<String> {
    Some(format!("auth.oauth_grant:{id}"))
}

/// A person's approval of an MCP client.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = OAuthGrant)]
pub(crate) struct GrantView {
    pub id: Uuid,
    /// The person who approved.
    pub user_id: Uuid,
    /// The client: for a metadata-document client, the URL of the document.
    pub client_id: String,
    /// Host of `client_id`.
    pub client_host: String,
    /// Name the client gave itself when approved (unverified).
    pub client_name: String,
    /// `cimd` or `dynamic`.
    pub registration: String,
    /// The resource the client uses the access for.
    pub resource: String,
    /// The permissions granted; the client never has more than the person currently has.
    pub permissions: Vec<String>,
    pub created_at: Timestamp,
    pub expires_at: Timestamp,
    /// Last time the client used it (to the minute); `null`: never.
    pub last_used_at: Option<Timestamp>,
    /// When it was revoked; `null`: still active (until `expires_at`).
    pub revoked_at: Option<Timestamp>,
}

impl From<grant::Model> for GrantView {
    fn from(row: grant::Model) -> Self {
        GrantView {
            id: row.id,
            user_id: row.user_id,
            client_host: host_of(&row.client_id),
            permissions: row.permissions(),
            client_id: row.client_id,
            client_name: row.client_name,
            registration: row.registration,
            resource: row.resource,
            created_at: row.created_at,
            expires_at: row.expires_at,
            last_used_at: row.last_used_at,
            revoked_at: row.revoked_at,
        }
    }
}

fn grant_not_found(id: Uuid) -> Error {
    Error::not_found("auth.oauth_grant_not_found", "There is no client approval with id `{id}`.")
        .with_param("id", id)
}

impl OAuthService {
    /// Grant của khách, mới nhất trước; `user`: chỉ của một người.
    pub(crate) async fn list(
        &self,
        tenant: &TenantContext,
        user: Option<Uuid>,
        page: &PageRequest,
    ) -> Result<Page<GrantView>, Error> {
        let mut select = grant::Entity::find().scoped(tenant);
        if let Some(user) = user {
            select = select.filter(grant::Column::UserId.eq(user));
        }
        let page =
            fetch_page(select, grant::Column::Id, |row| row.id, page, &tenant.db().read()).await?;
        Ok(page.map(GrantView::from))
    }

    /// Thu hồi grant `id`; `own`: chỉ grant của chính người gọi (không thì cần quyền quản trị, kiểm ở handler).
    /// Đã thu hồi: giữ nguyên (gọi lại an toàn).
    pub(crate) async fn revoke(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        own: bool,
    ) -> Result<GrantView, Error> {
        require_person(principal)?;
        let row = grant::Entity::find_by_id((tenant.id().as_uuid(), id))
            .scoped(tenant)
            .one(&tenant.db().read())
            .await?
            .filter(|row| !own || row.user_id == principal.user_id())
            .ok_or_else(|| grant_not_found(id))?;
        if row.revoked_at.is_some() {
            return Ok(row.into());
        }
        let transaction = tenant.db().begin_write().await?;
        revoke_in(&transaction, tenant, &self.outbox, Actor::from(principal), id, "revoked")
            .await?;
        transaction.commit().await?;
        let row = grant::Entity::find_by_id((tenant.id().as_uuid(), id))
            .scoped(tenant)
            .one(&tenant.db().read())
            .await?
            .ok_or_else(|| grant_not_found(id))?;
        Ok(row.into())
    }
}

/// Thu hồi grant `id` do hệ thống (mã hay refresh token bị dùng lại, client tự thu hồi).
pub(crate) async fn revoke_grant(
    service: &OAuthService,
    tenant: &TenantContext,
    id: Uuid,
    reason: &'static str,
) -> Result<(), Error> {
    let transaction = tenant.db().begin_write().await?;
    revoke_in(&transaction, tenant, &service.outbox, Actor::system(), id, reason).await?;
    transaction.commit().await?;
    tracing::info!(grant_id = %id, reason, "OAuth grant revoked");
    Ok(())
}

/// Thu hồi một grant trong transaction của bên gọi: đánh dấu, xoá mọi token của nó, ghi event (một lần).
pub(crate) async fn revoke_in<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    outbox: &Outbox,
    actor: Actor,
    id: Uuid,
    reason: &'static str,
) -> Result<(), Error> {
    let updated = grant::Entity::update_many()
        .col_expr(grant::Column::RevokedAt, Expr::value(Some(Timestamp::now())))
        .filter(grant::Column::TenantId.eq(tenant.id().as_uuid()))
        .filter(grant::Column::Id.eq(id))
        .filter(grant::Column::RevokedAt.is_null())
        .exec(connection)
        .await?;
    token::Entity::delete_many()
        .filter(token::Column::TenantId.eq(tenant.id().as_uuid()))
        .filter(token::Column::GrantId.eq(id))
        .exec(connection)
        .await?;
    if updated.rows_affected == 1 {
        let event = events::OauthRevoked {
            before: None,
            after: Some(serde_json::json!({"grant_id": id, "reason": reason})),
        };
        outbox.record(connection, tenant, actor, subject(id), &event).await?;
    }
    Ok(())
}

/// Thu hồi mọi grant còn hiệu lực của người dùng `user_id` trong transaction của bên gọi (khoá, xoá, lấy lại tài
/// khoản, đăng xuất khắp nơi).
pub(crate) async fn revoke_user_grants_in<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    outbox: &Outbox,
    actor: Actor,
    user_id: Uuid,
) -> Result<(), Error> {
    let ids: Vec<Uuid> = grant::Entity::find()
        .filter(grant::Column::UserId.eq(user_id))
        .filter(grant::Column::RevokedAt.is_null())
        .scoped(tenant)
        .all(connection)
        .await?
        .into_iter()
        .map(|row| row.id)
        .collect();
    for id in ids {
        revoke_in(connection, tenant, outbox, actor, id, "account").await?;
    }
    Ok(())
}
