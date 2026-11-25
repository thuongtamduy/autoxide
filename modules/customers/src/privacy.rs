//! Dữ liệu cá nhân trong hồ sơ khách (Luật 91/2025/QH15, NĐ 356/2025 — cần đối chiếu văn bản gốc):
//! bản xuất cho chính người mua, và xoá trường cá nhân khi tài khoản bị xoá — giữ con số tổng hợp
//! (tổng chi, số đơn) và lịch sử đơn không mang tên / số cho báo cáo và sổ cái. Nhận lại event cũng
//! không sao: lần sau không còn gì để đổi.

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::privacy::{PersonalData, deleted_user};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::time::Date;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::entity::{
    cart_reminder, group_member, order_fact, profile, recent_view, wishlist_alert_batch,
    wishlist_alert_queue, wishlist_alert_setting, wishlist_item,
};

#[async_trait::async_trait]
impl PersonalData for CustomerPrivacy {
    async fn export(&self, tenant: &TenantContext, user_id: Uuid) -> Result<Value, Error> {
        let read = tenant.database().read();
        let Some(row) = profile::Entity::find()
            .filter(profile::Column::UserId.eq(user_id))
            .scoped(tenant)
            .one(read)
            .await?
        else {
            return Ok(Value::Null);
        };
        let orders = order_fact::Entity::find()
            .filter(order_fact::Column::ProfileId.eq(row.id))
            .scoped(tenant)
            .order_by_asc(order_fact::Column::PlacedAt)
            .all(read)
            .await?;
        let groups = group_member::Entity::find()
            .filter(group_member::Column::ProfileId.eq(row.id))
            .scoped(tenant)
            .all(read)
            .await?;
        let wishlist = wishlist_item::Entity::find()
            .filter(wishlist_item::Column::ProfileId.eq(row.id))
            .scoped(tenant)
            .all(read)
            .await?;
        let views = recent_view::Entity::find()
            .filter(recent_view::Column::ProfileId.eq(row.id))
            .scoped(tenant)
            .all(read)
            .await?;
        let alerts = wishlist_alert_setting::Entity::find_by_id((tenant.id().as_uuid(), row.id))
            .one(read)
            .await?;
        Ok(json!({
            "wishlist_alerts": alerts.map(|alerts| json!({
                "back_in_stock": alerts.back_in_stock, "price_drop": alerts.price_drop,
                "updated_at": alerts.updated_at,
            })),
            "wishlist": wishlist.iter().map(|item| json!({
                "product_id": item.product_id, "variant_id": item.variant_id, "added_at": item.added_at,
            })).collect::<Vec<_>>(),
            "recently_viewed": views.iter().map(|view| json!({
                "product_id": view.product_id, "viewed_at": view.viewed_at,
            })).collect::<Vec<_>>(),
            "profile": {
                "id": row.id,
                "full_name": row.full_name,
                "phone": row.phone,
                "email": row.email,
                "birthday": row.birthday,
                "gender": row.gender,
                "tier": row.tier_code,
                "orders_count": row.orders_count,
                "total_spent_minor": row.total_spent_minor,
                "currency": row.currency,
                "custom_fields": serde_json::from_str::<Value>(&row.custom_fields).unwrap_or(Value::Null),
                "created_at": row.created_at,
            },
            "orders": orders.iter().map(|order| json!({
                "order_id": order.order_id,
                "number": order.number,
                "status": order.status,
                "net_minor": order.net_minor,
                "refunded_minor": order.refunded_minor,
                "currency": order.currency,
                "placed_at": order.placed_at,
            })).collect::<Vec<_>>(),
            "groups": groups.iter().map(|member| member.group_id).collect::<Vec<_>>(),
        }))
    }
}

/// Bản xuất và bên nghe `privacy::USER_DELETED`: chỉ cần database của khách, nên dùng được cả khi module
/// đang tắt (chế độ "nghỉ", bước 70 P9).
pub(crate) struct CustomerPrivacy;

#[async_trait::async_trait]
impl EventHandler for CustomerPrivacy {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let Some(user_id) = deleted_user(event) else {
            return Ok(());
        };
        let tenant = &event.tenant;
        let transaction = tenant.database().begin_write().await?;
        if let Some(row) = profile::Entity::find()
            .filter(profile::Column::UserId.eq(user_id))
            .scoped(tenant)
            .one(&transaction)
            .await?
        {
            // Yêu thích, vừa xem, nhắc giỏ đang chờ là dữ liệu hành vi của người đó: xoá.
            wishlist_item::Entity::delete_many()
                .filter(wishlist_item::Column::ProfileId.eq(row.id))
                .scoped(tenant)
                .exec(&transaction)
                .await?;
            recent_view::Entity::delete_many()
                .filter(recent_view::Column::ProfileId.eq(row.id))
                .scoped(tenant)
                .exec(&transaction)
                .await?;
            cart_reminder::Entity::delete_many()
                .filter(cart_reminder::Column::ProfileId.eq(row.id))
                .scoped(tenant)
                .exec(&transaction)
                .await?;
            // Báo yêu thích: cài đặt, hàng đợi, lô đã gửi (gắn với tài khoản).
            wishlist_alert_setting::Entity::delete_many()
                .filter(wishlist_alert_setting::Column::ProfileId.eq(row.id))
                .scoped(tenant)
                .exec(&transaction)
                .await?;
            wishlist_alert_queue::Entity::delete_many()
                .filter(wishlist_alert_queue::Column::ProfileId.eq(row.id))
                .scoped(tenant)
                .exec(&transaction)
                .await?;
            wishlist_alert_batch::Entity::delete_many()
                .filter(wishlist_alert_batch::Column::ProfileId.eq(row.id))
                .scoped(tenant)
                .exec(&transaction)
                .await?;
        }
        // Bỏ mọi trường nhận diện; hồ sơ còn lại là một dòng ẩn danh (tổng chi, số đơn).
        profile::Entity::update_many()
            .col_expr(profile::Column::UserId, Expr::value(Option::<Uuid>::None))
            .col_expr(profile::Column::PhoneKey, Expr::value(Option::<String>::None))
            .col_expr(profile::Column::Phone, Expr::value(Option::<String>::None))
            .col_expr(profile::Column::Email, Expr::value(Option::<String>::None))
            .col_expr(profile::Column::EmailLower, Expr::value(Option::<String>::None))
            .col_expr(profile::Column::FullName, Expr::value(Option::<String>::None))
            .col_expr(profile::Column::FullNameSearch, Expr::value(Option::<String>::None))
            .col_expr(profile::Column::Birthday, Expr::value(Option::<Date>::None))
            .col_expr(profile::Column::Gender, Expr::value(Option::<String>::None))
            .col_expr(profile::Column::BirthdayMd, Expr::value(Option::<String>::None))
            .col_expr(profile::Column::CustomFields, Expr::value("{}"))
            .col_expr(profile::Column::UpdatedAt, Expr::value(crate::service::now()))
            .filter(profile::Column::UserId.eq(user_id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }
}
