//! Token SCIM của khách: tạo (trả secret một lần), liệt kê, thu hồi, và xác thực request SCIM.
//! Xoay vòng: tạo token mới, đổi ở hệ thống danh tính, rồi thu hồi token cũ.

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::secret as token;
use autoxide_kernel::tenancy::{ScopeExt as _, TenantContext, TenantId, ensure_serving};
use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait as _, ColumnTrait as _, EntityTrait as _, PaginatorTrait as _,
    QueryFilter as _, QueryOrder as _,
};
use serde_json::json;
use uuid::Uuid;

use super::entity::token as entity;
use super::error::ScimError;
use super::{SCIM_FEATURE, ScimCaller, ScimService};
use crate::events;
use crate::permissions::SCIM_MANAGE;

/// Phần đầu của mọi token SCIM; sau nó là ID khách (32 chữ hex), `_`, phần ngẫu nhiên.
pub(crate) const TOKEN_PREFIX: &str = "axscim1_";
/// Số byte ngẫu nhiên của token (256 bit).
const TOKEN_BYTES: usize = 32;
/// Nhiều token nhất mỗi khách: đủ để xoay vòng, không để token cũ bị quên.
const MAX_TOKENS: u64 = 5;
/// Ghi `last_used_at` thưa nhất mỗi chừng này: không ghi database ở mọi request SCIM.
const TOUCH_EVERY_MINUTES: i64 = 5;
const NAME_MAX_LENGTH: usize = 100;

/// Một token SCIM, không có secret.
pub(crate) type ScimToken = entity::Model;

/// Token vừa tạo: secret chỉ có ở đây, một lần.
pub(crate) struct CreatedScimToken {
    pub token: ScimToken,
    pub secret: String,
}

fn now() -> Timestamp {
    Timestamp::now()
}

pub(crate) fn token_not_found(id: Uuid) -> Error {
    Error::not_found("auth.scim_token_not_found", "There is no SCIM token with id `{id}`.")
        .with_param("id", id)
}

/// Người tạo token còn làm token có hiệu lực không (token mạnh đúng bằng người tạo LÚC DÙNG,
/// D-20261125-scim-grant-cover): khác `Active` thì đồng bộ sẽ bị từ chối — phải xoay token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CreatorStatus {
    /// Người tạo còn hoạt động và còn `auth.scim.manage`.
    Active,
    /// Người tạo bị khoá (hay không còn là nhân sự).
    Disabled,
    /// Người tạo đã bị xoá.
    Deleted,
    /// Người tạo còn hoạt động nhưng không còn `auth.scim.manage`.
    MissingPermission,
}

impl ScimService {
    /// Tình trạng người tạo của từng token (`id` token → tình trạng). Khách có tối đa
    /// [`MAX_TOKENS`] token nên đọc từng người tạo một.
    pub(crate) async fn creator_statuses(
        &self,
        tenant: &TenantContext,
        tokens: &[ScimToken],
    ) -> Result<std::collections::HashMap<Uuid, CreatorStatus>, Error> {
        let mut by_creator = std::collections::HashMap::new();
        let mut statuses = std::collections::HashMap::new();
        for token in tokens {
            let status = match by_creator.get(&token.created_by) {
                Some(status) => *status,
                None => {
                    let status = match self.roles.user_principal(tenant, token.created_by).await? {
                        None => CreatorStatus::Deleted,
                        Some((user, _))
                            if !user.is_active()
                                || user.realm != autoxide_kernel::access::Realm::Staff =>
                        {
                            CreatorStatus::Disabled
                        }
                        Some((_, principal)) if !principal.has(&SCIM_MANAGE) => {
                            CreatorStatus::MissingPermission
                        }
                        Some(_) => CreatorStatus::Active,
                    };
                    by_creator.insert(token.created_by, status);
                    status
                }
            };
            statuses.insert(token.id, status);
        }
        Ok(statuses)
    }

    /// Các token của khách, mới nhất trước.
    pub(crate) async fn tokens(&self, tenant: &TenantContext) -> Result<Vec<ScimToken>, Error> {
        Ok(entity::Entity::find()
            .scoped(tenant)
            .order_by_desc(entity::Column::Id)
            .all(tenant.database().read())
            .await?)
    }

    /// Tạo token mới tên `name`; cần tính năng `auth.scim` của gói.
    pub(crate) async fn create_token(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        name: &str,
    ) -> Result<CreatedScimToken, Error> {
        // Token SCIM tạo và đổi email được mọi tài khoản mà người tạo bao trùm: một API key (dù chỉ
        // mang `auth.scim.manage`) không được tạo nó, như key không tạo được key (pentest 2026-11).
        crate::access::require_session(principal)?;
        let name = name.trim().to_owned();
        if !(1..=NAME_MAX_LENGTH).contains(&name.chars().count()) {
            return Err(Error::validation(vec![
                FieldError::new("name", "length", "must be 1 to {max} characters")
                    .with_param("max", NAME_MAX_LENGTH),
            ]));
        }
        self.entitlements.require_feature(tenant.id(), &SCIM_FEATURE).await?;
        let random = token::random_token(TOKEN_BYTES)?;
        let secret = format!("{TOKEN_PREFIX}{}_{random}", tenant.id().as_uuid().simple());
        let tail: String =
            random.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
        let row = entity::Model {
            tenant_id: tenant.id().as_uuid(),
            id: Uuid::now_v7(),
            name,
            prefix: format!("{TOKEN_PREFIX}…{tail}"),
            token_hash: token::sha256(&secret),
            created_by: principal.user_id(),
            created_at: now(),
            last_used_at: None,
        };
        let transaction = tenant.database().begin_write().await?;
        // Đếm trong transaction ghi: hai lần tạo đồng thời không vượt trần (SQLite một bên ghi;
        // Postgres khoá theo thứ tự nhờ dòng đếm không có — trần chỉ là chặn nhầm, không bảo mật).
        let existing = entity::Entity::find().scoped(tenant).count(&transaction).await?;
        if existing >= MAX_TOKENS {
            transaction.rollback().await?;
            return Err(Error::conflict(
                "auth.scim_token_limit",
                "This tenant already has {limit} SCIM tokens. Revoke one you no longer use first.",
            )
            .with_param("limit", MAX_TOKENS));
        }
        entity::ActiveModel::from(row.clone()).insert(&transaction).await?;
        let event = events::ScimTokenCreated {
            before: None,
            after: Some(json!({"id": row.id, "name": row.name, "prefix": row.prefix})),
        };
        let subject = Some(format!("auth.scim_token:{}", row.id));
        self.outbox.record(&transaction, tenant, Actor::from(principal), subject, &event).await?;
        transaction.commit().await?;
        tracing::info!(token_id = %row.id, "SCIM token created");
        Ok(CreatedScimToken { token: row, secret })
    }

    /// Thu hồi token `id`: request SCIM mang nó bị từ chối ngay.
    pub(crate) async fn revoke_token(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        let transaction = tenant.database().begin_write().await?;
        let Some(row) = entity::Entity::find()
            .filter(entity::Column::Id.eq(id))
            .scoped(tenant)
            .one(&transaction)
            .await?
        else {
            transaction.rollback().await?;
            return Err(token_not_found(id));
        };
        let deleted = entity::Entity::delete_many()
            .filter(entity::Column::Id.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        if deleted.rows_affected == 0 {
            transaction.rollback().await?;
            return Err(token_not_found(id));
        }
        let event = events::ScimTokenRevoked {
            before: Some(json!({"id": row.id, "name": row.name, "prefix": row.prefix})),
            after: None,
        };
        let subject = Some(format!("auth.scim_token:{id}"));
        self.outbox.record(&transaction, tenant, Actor::from(principal), subject, &event).await?;
        transaction.commit().await?;
        tracing::info!(token_id = %id, "SCIM token revoked");
        Ok(())
    }

    /// Xác thực request SCIM từ header `Authorization`. Khách lấy từ chính token (như API key);
    /// token phải còn trong bảng của khách đó và gói của khách phải có `auth.scim`.
    pub(crate) async fn authenticate(
        &self,
        authorization: Option<&str>,
    ) -> Result<ScimCaller, ScimError> {
        let secret =
            crate::session::bearer_token(authorization).map_err(|_| ScimError::unauthorized())?;
        let tenant_id = tenant_of(secret).ok_or_else(ScimError::unauthorized)?;
        let tenant = match self.databases.tenant(tenant_id).await {
            Ok(tenant) => tenant,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Err(ScimError::unauthorized());
            }
            Err(error) => return Err(error.into()),
        };
        ensure_serving(&tenant)?;
        let row = entity::Entity::find()
            .filter(entity::Column::TokenHash.eq(token::sha256(secret)))
            .scoped(&tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(ScimError::unauthorized)?;
        self.entitlements.require_feature(tenant.id(), &SCIM_FEATURE).await?;
        let now = now();
        if row
            .last_used_at
            .is_none_or(|used| used < now - SignedDuration::from_mins(TOUCH_EVERY_MINUTES))
        {
            // Ghi lỗi thì chỉ ghi log: dấu "dùng lần cuối" không đáng làm hỏng lần đồng bộ.
            let touched = entity::Entity::update_many()
                .col_expr(entity::Column::LastUsedAt, Expr::value(Some(now)))
                .filter(entity::Column::Id.eq(row.id))
                .scoped(&tenant)
                .exec(tenant.database().write())
                .await;
            if let Err(error) = touched {
                tracing::warn!(token_id = %row.id, %error, "could not record the use of a SCIM token");
            }
        }
        tracing::debug!(token_id = %row.id, "SCIM request authenticated");
        Ok(ScimCaller { tenant, created_by: row.created_by })
    }
}

/// Thu hồi mọi token SCIM do người `user_id` tạo, trong `connection` (transaction khoá tài khoản),
/// mỗi token một event `auth.scim_token_revoked`. Token SCIM mạnh đúng bằng người tạo lúc dùng: tài
/// khoản bị khoá hết đường vào (đổi email qua SCIM, "không phải tôi"…) mà token nó tạo còn sống thì
/// kẻ đã giữ tài khoản vẫn hành động với quyền sau này của tài khoản (D-20261125-scim-sync-report).
pub(crate) async fn revoke_created_by_in<C: sea_orm::ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    outbox: &autoxide_kernel::event::Outbox,
    actor: Actor,
    user_id: Uuid,
) -> Result<(), Error> {
    let rows = entity::Entity::find()
        .filter(entity::Column::CreatedBy.eq(user_id))
        .scoped(tenant)
        .all(connection)
        .await?;
    for row in rows {
        entity::Entity::delete_many()
            .filter(entity::Column::Id.eq(row.id))
            .scoped(tenant)
            .exec(connection)
            .await?;
        let event = events::ScimTokenRevoked {
            before: Some(json!({"id": row.id, "name": row.name, "prefix": row.prefix})),
            after: None,
        };
        let subject = Some(format!("auth.scim_token:{}", row.id));
        outbox.record(connection, tenant, actor, subject, &event).await?;
        tracing::info!(token_id = %row.id, %user_id, "SCIM token revoked with its creator's credentials");
    }
    Ok(())
}

/// Khách ghi trong token SCIM; `None` khi chuỗi không phải token SCIM.
fn tenant_of(secret: &str) -> Option<TenantId> {
    let rest = secret.strip_prefix(TOKEN_PREFIX)?;
    let hex = rest.get(..32)?;
    let random = rest.get(32..)?.strip_prefix('_')?;
    if random.is_empty() || !hex.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')) {
        return None;
    }
    Uuid::try_parse(hex).ok().map(TenantId::from_uuid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_khach_tu_token() {
        let id = Uuid::now_v7();
        let secret = format!("{TOKEN_PREFIX}{}_abc", id.simple());
        assert_eq!(tenant_of(&secret), Some(TenantId::from_uuid(id)));
        assert_eq!(tenant_of(&format!("{TOKEN_PREFIX}{}_", id.simple())), None);
        assert_eq!(tenant_of("axk1_abc"), None);
        assert_eq!(tenant_of(&format!("{TOKEN_PREFIX}XYZ")), None);
    }
}
