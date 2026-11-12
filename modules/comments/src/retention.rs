//! Job dọn dẹp hằng ngày `comments.cleanup`:
//!
//! - bình luận chờ duyệt / spam quá `AUTOXIDE_COMMENTS_RETENTION_DAYS` bị xoá hẳn (bình luận gốc
//!   còn trả lời thì xoá mềm, để luồng giữ nguyên);
//! - dấu vết IP quá `AUTOXIDE_COMMENTS_IP_RETENTION_DAYS` bị xoá;
//! - nonce, bộ đếm, token đã hết hạn bị dọn.
//!
//! Chạy lại bao nhiêu lần cũng ra cùng kết quả.

use std::sync::Arc;
use std::time::Duration;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::{JobContext, JobHandler};
use autoxide_kernel::throttle::Throttle;
use chrono::{DateTime, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect};
use serde_json::json;
use uuid::Uuid;

use crate::authoring::{Deletion, soft_delete};
use crate::entity::{comment, report, subscription};
use crate::events::{self, Change};
use crate::service::{CommentsService, MODULE, Status, now, shift_counts};

pub(crate) const CLEANUP: &str = "comments.cleanup";
/// Số bình luận xét mỗi vòng.
const BATCH: u64 = 200;

/// Đã dọn những gì.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Cleaned {
    /// Xoá hẳn.
    pub deleted: u64,
    /// Xoá mềm (bình luận gốc còn trả lời).
    pub cleared: u64,
    pub fingerprints: u64,
}

pub(crate) struct Cleanup(pub Arc<CommentsService>);

#[async_trait::async_trait]
impl JobHandler for Cleanup {
    async fn run(&self, _: &JobContext) -> Result<(), Error> {
        self.0.cleanup(now()).await.map(|_| ())
    }
}

impl CommentsService {
    /// Dọn theo thời hạn, coi `now` là bây giờ (test truyền thời điểm tương lai).
    pub(crate) async fn cleanup(&self, now: DateTime<Utc>) -> Result<Cleaned, Error> {
        let mut cleaned = Cleaned::default();
        let before = now - chrono::Duration::days(i64::from(self.config.retention_days));
        let mut after = Uuid::nil();
        let mut ids = Vec::new();
        loop {
            let rows = comment::Entity::find()
                .filter(
                    comment::Column::Status
                        .is_in([Status::Pending.as_str(), Status::Spam.as_str()]),
                )
                .filter(comment::Column::CreatedAt.lt(before))
                .filter(comment::Column::Id.gt(after))
                .order_by_asc(comment::Column::Id)
                .limit(BATCH)
                .all(self.database.read())
                .await?;
            let Some(last) = rows.last() else { break };
            after = last.id;
            for row in rows {
                let transaction = self.database.begin_write().await?;
                let others = if row.depth == 0 {
                    comment::Entity::find()
                        .filter(comment::Column::RootId.eq(row.id))
                        .filter(comment::Column::Id.ne(row.id))
                        .count(&transaction)
                        .await?
                } else {
                    0
                };
                if others > 0 {
                    soft_delete(self, &transaction, &row, Deletion::Erased).await?;
                    cleaned.cleared += 1;
                } else {
                    shift_counts(&transaction, &row, Some(Status::of(&row)), None).await?;
                    report::Entity::delete_many()
                        .filter(report::Column::CommentId.eq(row.id))
                        .exec(&transaction)
                        .await?;
                    subscription::Entity::delete_many()
                        .filter(subscription::Column::CommentId.eq(row.id))
                        .exec(&transaction)
                        .await?;
                    self.tokens.forget(&transaction, &row.id.to_string()).await?;
                    comment::Entity::delete_by_id(row.id).exec(&transaction).await?;
                    cleaned.deleted += 1;
                }
                transaction.commit().await?;
                ids.push(row.id);
            }
        }
        if !ids.is_empty() {
            let change = Change {
                before: Some(
                    json!({"comment_ids": ids, "count": ids.len(), "reason": "retention"}),
                ),
                after: None,
            };
            self.outbox
                .record(
                    self.database.write(),
                    Actor::system(),
                    None,
                    &events::CommentsExpired(change),
                )
                .await?;
        }
        let ip_before = now - chrono::Duration::days(i64::from(self.config.ip_retention_days));
        let write = self.database.write();
        cleaned.fingerprints = comment::Entity::update_many()
            .col_expr(comment::Column::IpFingerprint, Expr::value(Option::<String>::None))
            .filter(comment::Column::IpFingerprint.is_not_null())
            .filter(comment::Column::CreatedAt.lt(ip_before))
            .exec(write)
            .await?
            .rows_affected;
        self.guard.purge_nonces(write, MODULE).await?;
        self.tokens.purge(write, now).await?;
        Throttle::new(self.database.clone(), MODULE, 1, Duration::from_secs(3600))?
            .purge(now - chrono::Duration::days(1))
            .await?;
        if cleaned != Cleaned::default() {
            tracing::info!(
                deleted = cleaned.deleted,
                cleared = cleaned.cleared,
                fingerprints = cleaned.fingerprints,
                "comments cleanup"
            );
        }
        Ok(cleaned)
    }
}
