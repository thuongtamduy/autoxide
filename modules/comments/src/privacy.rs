//! Dữ liệu cá nhân trong module `comments`.
//!
//! - Theo tài khoản: bình luận có `user_id`, tên hiển thị (`comments_profiles`), báo cáo đã gửi.
//!   Xuất qua `PersonalData`. Tài khoản bị xoá (`privacy::USER_DELETED`): mọi bình luận của người
//!   đó bị xoá mềm (giữ chỗ trong luồng), `user_id` bỏ trống, hồ sơ bị xoá, báo cáo bỏ
//!   `reporter_user_id`. Nhận lại cùng event thì không còn gì để làm (D-054).
//! - Theo email (khách, hay email nhận mail báo): `GET` / `DELETE /api/v1/comments/data?email=`
//!   (`moderation.rs`), permission `comments.export`.

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Delivered, EventHandler};
use autoxide_kernel::privacy::{PersonalData, deleted_user};
use autoxide_kernel::tenancy::TenantContext;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::entity::{comment, profile, report};
use crate::events::{self, Change};
use crate::posting::authoring::{Deletion, soft_delete};
use crate::service::{CommentsService, Status};
use crate::text;

#[async_trait::async_trait]
impl PersonalData for CommentsService {
    async fn export(&self, user_id: Uuid) -> Result<Value, Error> {
        let mut rows = comment::Entity::find()
            .filter(comment::Column::UserId.eq(user_id))
            .all(self.database.read())
            .await?;
        rows.sort_by_key(|row| row.id);
        let profile = profile::Entity::find_by_id(user_id).one(self.database.read()).await?;
        let reports = report::Entity::find()
            .filter(report::Column::ReporterUserId.eq(user_id))
            .all(self.database.read())
            .await?;
        Ok(json!({
            "profile": profile.map(|row| json!({"display_name": row.display_name, "updated_at": row.updated_at})),
            "comments": rows.iter().map(|row| json!({
                "id": row.id,
                "target_type": row.target_type,
                "target_id": row.target_id,
                "reply_to_id": row.reply_to_id,
                "status": row.status,
                "body": row.body,
                "body_html": text::to_html(&row.body, text::Format::from_column(&row.format)),
                "format": row.format,
                "email": row.author_email,
                "locale": row.locale,
                "edited_at": row.edited_at,
                "created_at": row.created_at,
            })).collect::<Vec<_>>(),
            "reports": reports.iter().map(|row| json!({
                "comment_id": row.comment_id,
                "reason": row.reason,
                "note": row.note,
                "created_at": row.created_at,
            })).collect::<Vec<_>>(),
        }))
    }
}

#[async_trait::async_trait]
impl EventHandler for CommentsService {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let Some(user_id) = deleted_user(event) else {
            return Ok(());
        };
        let count = self.forget_user(user_id).await?;
        tracing::info!(user_id = %user_id, count, "comments of a deleted user cleared");
        Ok(())
    }
}

impl CommentsService {
    /// Xoá dữ liệu của tài khoản `user_id` đã bị xoá. Trả số bình luận vừa xoá mềm.
    pub(crate) async fn forget_user(&self, user_id: Uuid) -> Result<usize, Error> {
        let transaction = self.database.begin_write().await?;
        let rows = comment::Entity::find()
            .filter(comment::Column::UserId.eq(user_id))
            .all(&transaction)
            .await?;
        let mut ids = Vec::new();
        for row in &rows {
            if Status::of(row) == Status::Deleted {
                continue;
            }
            soft_delete(self, &transaction, row, Deletion::Erased).await?;
            ids.push(row.id);
        }
        // Bình luận đã xoá từ trước vẫn còn `user_id`.
        comment::Entity::update_many()
            .col_expr(comment::Column::UserId, Expr::value(Option::<Uuid>::None))
            .filter(comment::Column::UserId.eq(user_id))
            .exec(&transaction)
            .await?;
        profile::Entity::delete_by_id(user_id).exec(&transaction).await?;
        // Báo cáo đã gửi giữ lại (người kiểm duyệt cần), nhưng không còn chỉ tới người đó.
        let reports = report::Entity::find()
            .filter(report::Column::ReporterUserId.eq(user_id))
            .all(&transaction)
            .await?;
        for found in reports {
            report::Entity::update_many()
                .col_expr(report::Column::ReporterUserId, Expr::value(Option::<Uuid>::None))
                .col_expr(report::Column::ReporterKey, Expr::value(format!("deleted:{}", found.id)))
                .filter(report::Column::Id.eq(found.id))
                .exec(&transaction)
                .await?;
        }
        if !ids.is_empty() {
            let change = Change {
                before: Some(
                    json!({"comment_ids": ids, "count": ids.len(), "reason": "user_deleted"}),
                ),
                after: None,
            };
            self.outbox
                .record(
                    &transaction,
                    &TenantContext::legacy_default(&self.database),
                    Actor::system(),
                    None,
                    &events::CommentsErased(change),
                )
                .await?;
        }
        transaction.commit().await?;
        Ok(ids.len())
    }
}
