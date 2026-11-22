//! Khoá sửa trang (bước 67, D-229): người mở trình sửa giữ khoá [`EDIT_LOCK_SECONDS`] giây, gọi lại
//! để gia hạn; người khác thấy ai đang sửa tới khi nào. Khoá CHỈ ĐỂ BÁO — ghi vẫn kiểm `If-Match`
//! (khoá hết hạn hay ai đó bỏ qua khoá thì phiên bản vẫn chặn ghi đè).
//!
//! Chiếm khoá là MỘT lệnh: `INSERT … ON CONFLICT (tenant_id, entry_id) DO UPDATE … WHERE` khoá cũ
//! đã hết hạn hay của chính người gọi. Hai người cùng chiếm: Postgres xếp hai lệnh trên khoá chính,
//! lệnh sau thấy dòng của lệnh trước và điều kiện sai nên không đổi gì; SQLite chỉ một bên ghi. Sau
//! đó đọc lại dòng để biết ai thắng.

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter, Set};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::entity::edit_lock::{self, Column, Entity};
use crate::permissions;
use crate::service::{ContentService, entry_not_found, now};

/// Thời gian giữ khoá mỗi lần chiếm hay gia hạn (2 phút, đặc tả bước 67).
pub(crate) const EDIT_LOCK_SECONDS: i64 = 120;

/// Who is editing an entry, and until when.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct EditLock {
    /// The entry being edited.
    pub entry_id: Uuid,
    /// The user holding the lock.
    pub user_id: Uuid,
    /// The API key holding the lock, when the holder works through one (e.g. an agent).
    pub api_key_id: Option<Uuid>,
    /// When the holder first took the lock.
    #[schema(value_type = String, format = DateTime)]
    pub acquired_at: Timestamp,
    /// When the lock lapses unless the holder renews it (`POST` again).
    #[schema(value_type = String, format = DateTime)]
    pub expires_at: Timestamp,
    /// `true` when you hold the lock (same user and same API key).
    pub mine: bool,
}

fn view(row: edit_lock::Model, principal: &Principal) -> EditLock {
    let mine = holds(&row, principal);
    EditLock {
        entry_id: row.entry_id,
        user_id: row.user_id,
        api_key_id: row.api_key_id,
        acquired_at: row.acquired_at,
        expires_at: row.expires_at,
        mine,
    }
}

fn holds(row: &edit_lock::Model, principal: &Principal) -> bool {
    row.user_id == principal.user_id() && row.api_key_id == principal.api_key_id()
}

/// 409 `content.edit_locked`: người khác đang giữ khoá.
pub(crate) fn locked(lock: &EditLock) -> Error {
    Error::conflict(
        "content.edit_locked",
        "User {user} is editing this page until {until}; try again after that, or ask them to \
         close the editor.",
    )
    .with_param("user", lock.user_id)
    .with_param("until", lock.expires_at.to_string())
}

/// Điều kiện "khoá này của người gọi" (cùng người, cùng API key — NULL so bằng `IS NULL`).
fn owned_by(principal: &Principal) -> Condition {
    let key = match principal.api_key_id() {
        Some(key) => Column::ApiKeyId.eq(key),
        None => Column::ApiKeyId.is_null(),
    };
    Condition::all().add(Column::UserId.eq(principal.user_id())).add(key)
}

/// Khoá còn hiệu lực của mục `id` (không kiểm quyền).
async fn current(
    tenant: &TenantContext,
    principal: &Principal,
    id: Uuid,
) -> Result<Option<EditLock>, Error> {
    let row = Entity::find_by_id((tenant.id().as_uuid(), id))
        .scoped(tenant)
        .one(tenant.database().read())
        .await?;
    Ok(row.filter(|row| row.expires_at > now()).map(|row| view(row, principal)))
}

impl ContentService {
    /// Khoá đang có hiệu lực của mục `id`, nếu có. Cần `content.read` (như đọc mục).
    pub(crate) async fn edit_lock(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Option<EditLock>, Error> {
        self.get(tenant, principal, id).await?;
        current(tenant, principal, id).await
    }

    /// Chiếm hay gia hạn khoá sửa của mục `id`. Người khác đang giữ (chưa hết hạn) → 409
    /// `content.edit_locked` kèm người giữ và giờ hết hạn. Cần quyền sửa mục.
    pub(crate) async fn acquire_edit_lock(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<EditLock, Error> {
        self.require_editable(tenant, principal, id).await?;
        let now = now();
        let row = edit_lock::ActiveModel {
            tenant_id: Set(tenant.id().as_uuid()),
            entry_id: Set(id),
            user_id: Set(principal.user_id()),
            api_key_id: Set(principal.api_key_id()),
            acquired_at: Set(now),
            expires_at: Set(now + SignedDuration::from_secs(EDIT_LOCK_SECONDS)),
        };
        // Chỉ đè dòng cũ khi nó đã hết hạn hay là của chính người gọi. Gia hạn giữ `acquired_at`
        // cũ; chiếm lại khoá hết hạn của người khác thì `acquired_at` mới.
        // `ColumnTrait` ghi cột kèm tên bảng: trong `DO UPDATE … WHERE` đó là dòng đang có.
        let takeover = Column::ExpiresAt.lte(now);
        let renewal = owned_by(principal);
        let acquired_at = Expr::case(takeover.clone(), Expr::val(now))
            .finally(Expr::col((Entity, Column::AcquiredAt)));
        Entity::insert(row)
            .on_conflict(
                OnConflict::columns([Column::TenantId, Column::EntryId])
                    .update_columns([Column::UserId, Column::ApiKeyId, Column::ExpiresAt])
                    .value(Column::AcquiredAt, acquired_at)
                    .action_cond_where(Condition::any().add(takeover).add(renewal))
                    .to_owned(),
            )
            .exec_without_returning(tenant.database().write())
            .await?;
        let stored = Entity::find_by_id((tenant.id().as_uuid(), id))
            .scoped(tenant)
            .one(tenant.database().write())
            .await?
            .ok_or_else(|| entry_not_found(id))?;
        let lock = view(stored, principal);
        if lock.mine { Ok(lock) } else { Err(locked(&lock)) }
    }

    /// Nhả khoá sửa của mục `id`. Không có khoá hay khoá đã hết hạn: không làm gì. Người khác đang
    /// giữ → 409 `content.edit_locked` (không nhả hộ người khác).
    pub(crate) async fn release_edit_lock(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        self.require_editable(tenant, principal, id).await?;
        let removed = Entity::delete_many()
            .filter(Column::EntryId.eq(id))
            .filter(owned_by(principal))
            .scoped(tenant)
            .exec(tenant.database().write())
            .await?;
        if removed.rows_affected > 0 {
            return Ok(());
        }
        match current(tenant, principal, id).await? {
            Some(lock) if !lock.mine => Err(locked(&lock)),
            _ => Ok(()),
        }
    }

    /// Mục `id` có và người gọi sửa được nó.
    async fn require_editable(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        permissions::require_create(principal)?;
        let entry = self.load_checked(tenant, id, None).await?;
        permissions::require_write(principal, &entry)?;
        permissions::require_org(principal, tenant, &entry, permissions::Need::Write).await
    }
}
