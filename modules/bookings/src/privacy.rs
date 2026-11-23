//! Dữ liệu cá nhân trong lịch hẹn (Luật Bảo vệ dữ liệu cá nhân; rà soát bảo mật 2026-11).
//!
//! - Bản xuất của người dùng có các lịch họ đặt bằng tài khoản (dịch vụ, giờ, trạng thái, tên / số /
//!   email đã khai, tiền cọc).
//! - Tài khoản bị xoá: lịch vẫn giữ (tiền cọc, hoàn tiền là chứng từ), nhưng bỏ tên, số, email, ghi
//!   chú và mối nối tới tài khoản; nhân viên gắn tài khoản đó thôi gắn (tài nguyên vẫn còn cho lịch
//!   của cửa hàng).

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::privacy::{PersonalData, deleted_user};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::entity::{booking, resource};
use crate::service::now;

/// Phần dữ liệu cá nhân của `bookings`.
pub(crate) struct BookingPrivacy;

#[async_trait::async_trait]
impl PersonalData for BookingPrivacy {
    async fn export(&self, tenant: &TenantContext, user_id: Uuid) -> Result<Value, Error> {
        let rows = booking::Entity::find()
            .filter(booking::Column::CustomerUserId.eq(user_id))
            .scoped(tenant)
            .order_by_asc(booking::Column::Id)
            .all(tenant.database().read())
            .await?;
        let bookings: Vec<Value> = rows
            .iter()
            .map(|row| {
                json!({
                    "id": row.id,
                    "service_id": row.service_id,
                    "starts_at": row.starts_at,
                    "ends_at": row.ends_at,
                    "status": row.status,
                    "seats": row.seats,
                    "name": row.customer_name,
                    "phone": row.customer_phone,
                    "email": row.customer_email,
                    "note": row.note,
                    "price_minor": row.price_minor,
                    "deposit_minor": row.deposit_minor,
                    "currency": row.currency,
                    "created_at": row.created_at,
                })
            })
            .collect();
        Ok(json!({ "bookings": bookings }))
    }
}

/// Tài khoản bị xoá: xoá thông tin cá nhân khỏi lịch của người đó và gỡ nhân viên khỏi tài khoản,
/// trong khách của event. Nhận lại event không sao: lần sau không còn dòng nào khớp.
#[async_trait::async_trait]
impl EventHandler for BookingPrivacy {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let Some(user_id) = deleted_user(event) else {
            return Ok(());
        };
        let tenant = &event.tenant;
        let transaction = tenant.database().begin_write().await?;
        let now = now();
        let bookings = booking::Entity::update_many()
            .col_expr(booking::Column::CustomerName, Expr::value(""))
            .col_expr(booking::Column::CustomerPhone, Expr::value(Option::<String>::None))
            .col_expr(booking::Column::CustomerEmail, Expr::value(Option::<String>::None))
            .col_expr(booking::Column::Note, Expr::value(""))
            .col_expr(booking::Column::CustomerUserId, Expr::value(Option::<Uuid>::None))
            .col_expr(booking::Column::UpdatedAt, Expr::value(now))
            .filter(booking::Column::CustomerUserId.eq(user_id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        let staff = resource::Entity::update_many()
            .col_expr(resource::Column::UserId, Expr::value(Option::<Uuid>::None))
            .col_expr(resource::Column::UpdatedAt, Expr::value(now))
            .filter(resource::Column::UserId.eq(user_id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        transaction.commit().await?;
        tracing::info!(
            user_id = %user_id,
            bookings = bookings.rows_affected,
            staff = staff.rows_affected,
            "booking data of a deleted user removed"
        );
        Ok(())
    }
}
