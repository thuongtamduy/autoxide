//! Dữ liệu cá nhân module giữ (Luật Bảo vệ dữ liệu cá nhân 91/2025/QH15 — cần đối chiếu): xuất cho
//! chủ tài khoản, xoá khi tài khoản bị xoá (event `auth.user_deleted`, nhận lại vô hại).

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::privacy::{PersonalData, deleted_user};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::entity::{device_token, inbox, message, preference, push_subscription};
use crate::service::NotificationsService;

/// Số dòng tối đa mỗi phần trong bản xuất (mới nhất trước).
const EXPORT_LIMIT: u64 = 1000;

pub(crate) struct NotificationsPersonalData;

#[async_trait::async_trait]
impl PersonalData for NotificationsPersonalData {
    async fn export(&self, tenant: &TenantContext, user_id: Uuid) -> Result<Value, Error> {
        let read = tenant.database().read();
        let inbox = inbox::Entity::find()
            .scoped(tenant)
            .filter(inbox::Column::UserId.eq(user_id))
            .order_by_desc(inbox::Column::Id)
            .limit(EXPORT_LIMIT)
            .all(read)
            .await?;
        let preferences = preference::Entity::find()
            .scoped(tenant)
            .filter(preference::Column::UserId.eq(user_id))
            .all(read)
            .await?;
        let browsers = push_subscription::Entity::find()
            .scoped(tenant)
            .filter(push_subscription::Column::UserId.eq(user_id))
            .all(read)
            .await?;
        let devices = device_token::Entity::find()
            .scoped(tenant)
            .filter(device_token::Column::UserId.eq(user_id))
            .all(read)
            .await?;
        let messages = message::Entity::find()
            .scoped(tenant)
            .filter(message::Column::UserId.eq(user_id))
            .order_by_desc(message::Column::Id)
            .limit(EXPORT_LIMIT)
            .all(read)
            .await?;
        // Không xuất secret: khoá `auth` của trình duyệt và token thiết bị.
        Ok(json!({
            "inbox": inbox.iter().map(|row| json!({
                "id": row.id, "kind": row.kind, "title": row.title, "body": row.body,
                "link": row.link, "read_at": row.read_at, "created_at": row.created_at,
            })).collect::<Vec<_>>(),
            "preferences": preferences.iter().map(|row| json!({
                "target": row.target, "channel": row.channel, "enabled": row.enabled,
            })).collect::<Vec<_>>(),
            "push_subscriptions": browsers.iter().map(|row| json!({
                "id": row.id, "endpoint": row.endpoint, "user_agent": row.user_agent, "created_at": row.created_at,
            })).collect::<Vec<_>>(),
            "devices": devices.iter().map(|row| json!({
                "id": row.id, "platform": row.platform, "app_id": row.app_id, "created_at": row.created_at,
            })).collect::<Vec<_>>(),
            "messages": messages.iter().map(|row| json!({
                "id": row.id, "kind": row.kind, "channel": row.channel, "status": row.status,
                "address": row.address, "created_at": row.created_at, "sent_at": row.sent_at,
            })).collect::<Vec<_>>(),
        }))
    }
}

impl NotificationsService {
    /// Xoá dữ liệu của người dùng `user` đã bị xoá: hộp thư, tuỳ chọn, đăng ký đẩy; nhật ký gửi giữ
    /// dòng (thống kê) nhưng bỏ địa chỉ và giá trị. Gọi lại vô hại.
    // Không dùng `self`: module đang tắt (chế độ "nghỉ", bước 70 P9) gọi được mà không có service.
    pub(crate) async fn erase_user(tenant: &TenantContext, user: Uuid) -> Result<(), Error> {
        let transaction = tenant.database().begin_write().await?;
        inbox::Entity::delete_many()
            .filter(inbox::Column::UserId.eq(user))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        preference::Entity::delete_many()
            .filter(preference::Column::UserId.eq(user))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        push_subscription::Entity::delete_many()
            .filter(push_subscription::Column::UserId.eq(user))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        device_token::Entity::delete_many()
            .filter(device_token::Column::UserId.eq(user))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        message::Entity::update_many()
            .col_expr(message::Column::Address, Expr::value(Option::<String>::None))
            .col_expr(message::Column::Params, Expr::value("{}"))
            .col_expr(message::Column::Status, Expr::value("canceled"))
            .filter(message::Column::UserId.eq(user))
            .filter(message::Column::Status.is_in(["queued", "deferred"]))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        message::Entity::update_many()
            .col_expr(message::Column::Address, Expr::value(Option::<String>::None))
            .col_expr(message::Column::Params, Expr::value("{}"))
            .filter(message::Column::UserId.eq(user))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }
}

/// Nghe `auth.user_deleted` — cả khi module đang tắt.
pub(crate) struct UserDeletedListener;

#[async_trait::async_trait]
impl EventHandler for UserDeletedListener {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        match deleted_user(event) {
            Some(user) => NotificationsService::erase_user(&event.tenant, user).await,
            None => Ok(()),
        }
    }
}
