//! Xoay vòng API key không gián đoạn (bước 54 P4): tạo key mới cùng tên, permission, nút, dải IP và
//! loại với key cũ; key cũ còn chạy thêm `overlap_hours` giờ (mặc định 24) rồi hết hạn, đủ để máy
//! đổi sang key mới mà không có lúc nào không gọi được.

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use chrono::{DateTime, Duration, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
use uuid::Uuid;

use super::entity::api_key;
use super::service::{
    ApiKeyService, CreatedApiKey, KeyKind, NewApiKey, Reauth, api_key_not_found, permissions_of,
};

/// Số giờ dài nhất key cũ còn chạy song song với key mới.
pub(super) const MAX_OVERLAP_HOURS: u32 = 72;

/// Key cũ bị thay: `id`, hết hạn lúc `until`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Replacing {
    pub id: Uuid,
    pub until: DateTime<Utc>,
    /// Trạng thái của key cũ: key mới giữ nguyên (cấu hình y hệt, không cần duyệt lại).
    pub status: &'static str,
}

/// Yêu cầu xoay vòng một key.
#[derive(Debug, Clone)]
pub(crate) struct Rotation {
    /// Số giờ key cũ còn chạy (0 tới [`MAX_OVERLAP_HOURS`]); 0: thôi chạy ngay.
    pub overlap_hours: u32,
    /// Hạn của key mới; `None`: hạn dài nhất cho phép.
    pub expires_at: Option<DateTime<Utc>>,
    /// Xác nhận lại như lúc tạo key: mật khẩu hiện tại, hay một yếu tố thứ hai.
    pub current_password: String,
    pub second_factor: Option<Reauth>,
}

impl ApiKeyService {
    /// Thay key `id` của bên gọi bằng key mới; trả key mới kèm secret (chỉ có ở đây).
    pub(crate) async fn rotate(
        &self,
        tenant: &TenantContext,
        caller: &Principal,
        id: Uuid,
        rotation: Rotation,
    ) -> Result<CreatedApiKey, Error> {
        let read = tenant.database().read();
        let old = api_key::Entity::find_by_id(id)
            .filter(api_key::Column::UserId.eq(caller.user_id()))
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(|| api_key_not_found(id))?;
        if rotation.overlap_hours > MAX_OVERLAP_HOURS {
            return Err(Error::validation(vec![
                FieldError::new(
                    "overlap_hours",
                    "range",
                    "must be between 0 and {max_overlap_hours}",
                )
                .with_param("max_overlap_hours", MAX_OVERLAP_HOURS),
            ]));
        }
        let permissions = permissions_of(read, tenant, [old.id]).await?.remove(&old.id);
        let allowed_ips = super::rules::parse_stored(old.allowed_ips.as_deref())
            .iter()
            .map(ToString::to_string)
            .collect();
        let input = NewApiKey {
            name: old.name.clone(),
            kind: autoxide_kernel::access::ActorKind::of_key(&old.kind),
            permissions: permissions.unwrap_or_default(),
            // Key cũ không gắn nút (dữ liệu rất cũ) thì key mới cũng không.
            org_id: old.org_id,
            expires_at: rotation.expires_at,
            allowed_ips,
            current_password: rotation.current_password,
            second_factor: rotation.second_factor,
        };
        let until = Utc::now() + Duration::hours(i64::from(rotation.overlap_hours));
        let status = if old.status == super::approval::PENDING {
            super::approval::PENDING
        } else {
            super::approval::ACTIVE
        };
        let replacing = Replacing { id, until, status };
        self.create_replacing(tenant, caller, input, Some(replacing)).await
    }
}

/// Đặt hạn cho key cũ trong transaction tạo key mới: `until`, trừ khi nó vốn hết hạn sớm hơn. Key
/// đã bị thu hồi (xoá) giữa chừng thì hỏng cả lần xoay: không tạo key thay cho thứ không còn.
pub(super) async fn retire<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    owner: Uuid,
    replacing: &Replacing,
) -> Result<(), Error> {
    let row = api_key::Entity::find_by_id(replacing.id)
        .filter(api_key::Column::UserId.eq(owner))
        .scoped(tenant)
        .one(transaction)
        .await?
        .ok_or_else(|| api_key_not_found(replacing.id))?;
    let until = row.expires_at.map_or(replacing.until, |at| at.min(replacing.until));
    api_key::Entity::update_many()
        .col_expr(api_key::Column::ExpiresAt, Expr::value(Some(until)))
        .filter(api_key::Column::Id.eq(replacing.id))
        .scoped(tenant)
        .exec(transaction)
        .await?;
    Ok(())
}
