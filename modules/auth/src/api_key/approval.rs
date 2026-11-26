//! Quản trị duyệt API key (bước 54 P4, như GitHub cho chủ tổ chức duyệt token chi tiết): khách bật
//! cấu hình `auth.api_keys.require_approval` thì key mới ở trạng thái `pending` — chưa dùng được —
//! tới khi người có `auth.api_keys.approve` duyệt. Người giữ quyền duyệt tạo key thì key chạy
//! ngay; key xoay vòng giữ trạng thái của key nó thay (cấu hình y hệt). Từ chối là thu hồi key.

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext, TenantSetting, parsed_setting};
use autoxide_kernel::time::Timestamp;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::json;
use uuid::Uuid;

use super::entity::api_key;
use super::service::{ApiKey, ApiKeyService, api_key_not_found, permissions_of, to_api_key};
use crate::events;

/// Giá trị cột `status`.
pub(crate) const ACTIVE: &str = "active";
pub(crate) const PENDING: &str = "pending";

/// Cấu hình theo khách: key mới cần quản trị duyệt.
pub(crate) const REQUIRE_APPROVAL: TenantSetting = TenantSetting::new(
    "auth.api_keys.require_approval",
    "`true`: a new API key waits for approval by someone with `auth.api_keys.approve` before it \
     works (keys made by those people work at once). `false` (default): keys work at once.",
)
.checked(check_flag);

fn check_flag(raw: &str) -> Result<(), FieldError> {
    parse_flag(raw).map(|_| ())
}

fn parse_flag(raw: &str) -> Result<bool, FieldError> {
    match raw.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(FieldError::new("value", "invalid", "must be `true` or `false`")),
    }
}

impl ApiKeyService {
    /// Trạng thái ban đầu của key do `creator` tạo ở khách `tenant`.
    pub(super) async fn initial_status(
        &self,
        tenant: &TenantContext,
        creator: &Principal,
    ) -> Result<&'static str, Error> {
        if creator.has(&crate::permissions::API_KEYS_APPROVE) {
            return Ok(ACTIVE);
        }
        let Some(settings) = &self.settings else { return Ok(ACTIVE) };
        let required =
            parsed_setting(settings.as_ref(), tenant, &REQUIRE_APPROVAL, parse_flag).await?;
        Ok(if required == Some(true) { PENDING } else { ACTIVE })
    }

    /// Duyệt key `id` đang chờ. Cần `auth.api_keys.approve` và một phiên đăng nhập: API key không
    /// duyệt được key (key lộ không tự mở đường cho key khác).
    pub(crate) async fn approve(
        &self,
        tenant: &TenantContext,
        caller: &Principal,
        id: Uuid,
    ) -> Result<ApiKey, Error> {
        caller.require(&crate::permissions::API_KEYS_APPROVE)?;
        if !caller.is_session() {
            return Err(Error::forbidden(
                "auth.api_key_cannot_approve_keys",
                "An API key cannot approve API keys. Sign in to approve keys.",
            ));
        }
        let now = Timestamp::now();
        let transaction = tenant.database().begin_write().await?;
        let row = api_key::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| api_key_not_found(id))?;
        // Điều kiện trạng thái lặp lại trong câu UPDATE: hai người duyệt cùng lúc thì một bên ghi.
        let updated = api_key::Entity::update_many()
            .col_expr(api_key::Column::Status, Expr::value(ACTIVE))
            .col_expr(api_key::Column::ApprovedBy, Expr::value(Some(caller.user_id())))
            .col_expr(api_key::Column::ApprovedAt, Expr::value(Some(now)))
            .filter(api_key::Column::Id.eq(id))
            .filter(api_key::Column::Status.eq(PENDING))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        if updated.rows_affected == 0 {
            transaction.rollback().await?;
            return Err(Error::conflict(
                "auth.api_key_not_pending",
                "The API key `{id}` does not wait for approval: it already works.",
            )
            .with_param("id", id));
        }
        let after = json!({"user_id": row.user_id, "name": row.name, "prefix": row.prefix});
        let event = events::ApiKeyApproved { before: None, after: Some(after) };
        let subject = Some(format!("auth.api_key:{id}"));
        self.outbox.record(&transaction, tenant, Actor::from(caller), subject, &event).await?;
        transaction.commit().await?;
        tracing::info!(api_key_id = %id, "API key approved");
        let read = tenant.database().read();
        let row = api_key::Entity::find_by_id(id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(|| api_key_not_found(id))?;
        let permissions = permissions_of(read, tenant, [id]).await?.remove(&id).unwrap_or_default();
        Ok(to_api_key(row, permissions))
    }
}

/// Lỗi khi dùng key còn chờ duyệt.
pub(super) fn pending() -> Error {
    Error::unauthenticated(
        "auth.api_key_pending",
        "This API key waits for an administrator's approval and does not work yet.",
    )
}
