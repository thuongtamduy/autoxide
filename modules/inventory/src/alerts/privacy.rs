//! Dữ liệu cá nhân của đăng ký báo có hàng lại (Luật Bảo vệ dữ liệu cá nhân; rà soát bảo mật
//! 2026-11): bản xuất của người dùng có các đăng ký gắn tài khoản của họ; tài khoản bị xoá thì xoá
//! các đăng ký đó (đăng ký chỉ phục vụ việc báo tin, không phải chứng từ cần giữ).
//!
//! Đăng ký của khách vãng lai (email / số điện thoại, không tài khoản) không gắn với người dùng nào:
//! họ dừng bằng link trong tin.

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::privacy::{PersonalData, deleted_user};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::entity::restock_alert;

/// Phần dữ liệu cá nhân của báo có hàng lại.
pub(crate) struct AlertPrivacy;

#[async_trait::async_trait]
impl PersonalData for AlertPrivacy {
    async fn export(&self, tenant: &TenantContext, user_id: Uuid) -> Result<Value, Error> {
        let rows = restock_alert::Entity::find()
            .filter(restock_alert::Column::UserId.eq(user_id))
            .scoped(tenant)
            .order_by_asc(restock_alert::Column::Id)
            .all(tenant.database().read())
            .await?;
        let alerts: Vec<Value> = rows
            .iter()
            .map(|row| {
                json!({
                    "variant_id": row.variant_id,
                    "store_id": (!row.store_key.is_nil()).then_some(row.store_key),
                    "status": row.status,
                    "created_at": row.created_at,
                    "confirmed_at": row.confirmed_at,
                    "sent_count": row.sent_count,
                })
            })
            .collect();
        Ok(json!({ "restock_alerts": alerts }))
    }
}

/// Tài khoản bị xoá: xoá các đăng ký gắn tài khoản đó, trong khách của event. Nhận lại event
/// không sao: lần sau không còn gì để xoá.
#[async_trait::async_trait]
impl EventHandler for AlertPrivacy {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let Some(user_id) = deleted_user(event) else {
            return Ok(());
        };
        let tenant = &event.tenant;
        let deleted = restock_alert::Entity::delete_many()
            .filter(restock_alert::Column::UserId.eq(user_id))
            .scoped(tenant)
            .exec(tenant.database().write())
            .await?;
        tracing::info!(user_id = %user_id, alerts = deleted.rows_affected, "restock alerts of a deleted user removed");
        Ok(())
    }
}
