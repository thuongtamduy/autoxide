//! Dữ liệu cá nhân trong phiếu (Luật 91/2025/QH15): bản xuất cho người dùng, và ẩn danh hoá khi
//! tài khoản bị xoá — giữ phiếu (số, loại, trạng thái, mốc thời gian) cho thống kê, bỏ người hỏi và
//! nội dung người đó viết. Nhận lại event cũng không sao: lần sau không còn gì để đổi.

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::privacy::{PersonalData, deleted_user};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::entity::{message, ticket};
use crate::service::SupportService;

#[async_trait::async_trait]
impl PersonalData for SupportService {
    async fn export(&self, tenant: &TenantContext, user_id: Uuid) -> Result<Value, Error> {
        let read = tenant.database().read();
        let tickets = ticket::Entity::find()
            .filter(ticket::Column::RequesterUserId.eq(user_id))
            .scoped(tenant)
            .order_by_asc(ticket::Column::Id)
            .all(read)
            .await?;
        let mut out = Vec::new();
        for ticket in tickets {
            let messages = message::Entity::find()
                .filter(message::Column::TicketId.eq(ticket.id))
                .filter(message::Column::AuthorUserId.eq(user_id))
                .scoped(tenant)
                .order_by_asc(message::Column::Id)
                .all(read)
                .await?;
            out.push(json!({
                "id": ticket.id,
                "number": ticket.number,
                "subject": ticket.subject,
                "status": ticket.status,
                "created_at": ticket.created_at,
                "messages": messages.iter().map(|message| json!({
                    "body": message.body,
                    "created_at": message.created_at,
                })).collect::<Vec<_>>(),
            }));
        }
        Ok(Value::Array(out))
    }
}

#[async_trait::async_trait]
impl EventHandler for SupportService {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let Some(user_id) = deleted_user(event) else {
            return Ok(());
        };
        let tenant = &event.tenant;
        let transaction = tenant.database().begin_write().await?;
        let tickets = ticket::Entity::find()
            .filter(ticket::Column::RequesterUserId.eq(user_id))
            .scoped(tenant)
            .all(&transaction)
            .await?;
        let ids: Vec<Uuid> = tickets.iter().map(|ticket| ticket.id).collect();
        if !ids.is_empty() {
            // Tin của người mua trong các phiếu đó: bỏ nội dung, giữ dòng (đếm, thời gian).
            message::Entity::update_many()
                .col_expr(message::Column::Body, Expr::value(""))
                .col_expr(message::Column::MediaIds, Expr::value("[]"))
                .col_expr(message::Column::AuthorUserId, Expr::value(Option::<Uuid>::None))
                .filter(message::Column::TicketId.is_in(ids.clone()))
                .filter(message::Column::AuthorKind.eq("customer"))
                .scoped(tenant)
                .exec(&transaction)
                .await?;
            ticket::Entity::update_many()
                .col_expr(ticket::Column::RequesterUserId, Expr::value(Option::<Uuid>::None))
                .col_expr(ticket::Column::RequesterEmail, Expr::value(Option::<String>::None))
                .col_expr(ticket::Column::RequesterPhone, Expr::value(Option::<String>::None))
                .col_expr(ticket::Column::RequesterName, Expr::value(""))
                .col_expr(ticket::Column::TokenHash, Expr::value(Option::<String>::None))
                .col_expr(ticket::Column::SatisfactionNote, Expr::value(Option::<String>::None))
                .filter(ticket::Column::Id.is_in(ids))
                .scoped(tenant)
                .exec(&transaction)
                .await?;
        }
        transaction.commit().await?;
        tracing::info!(user_id = %user_id, tickets = tickets.len(), "support tickets of a deleted user anonymized");
        Ok(())
    }
}
