//! Xin duyệt (D-109): bên viết — thường là AI agent chỉ có `content.write_own` — báo rằng bản
//! nháp đã sẵn sàng để một người xem và đăng. Chỉ là một cờ trên bản nháp kèm event
//! `content.review_requested` (website hay hệ thống thông báo nhận qua webhook); không có
//! trạng thái "đã duyệt / từ chối" — quy trình duyệt nhiều bước nằm ngoài phạm vi (bước 17).

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::entries::{Entry, summary};
use crate::events::{self, Change};
use crate::permissions;
use crate::service::{ContentService, now};

impl ContentService {
    /// Xin duyệt bản nháp của mục `id` (`withdraw`: rút lại). Cần `content.write`, hoặc
    /// `content.write_own` trên mục của mình. `If-Match` bắt buộc như mọi thay đổi mục.
    pub(crate) async fn set_review_request(
        &self,
        principal: &Principal,
        id: Uuid,
        expected: Option<DateTime<Utc>>,
        withdraw: bool,
    ) -> Result<Entry, Error> {
        permissions::require_create(principal)?;
        let entry = self.load_checked(id, expected).await?;
        permissions::require_write(principal, &entry)?;
        let now = now();
        let transaction = self.database.begin_write().await?;
        let current = self.reload(&transaction, id, expected.unwrap_or(entry.updated_at)).await?;
        let mut after = current.clone();
        if withdraw {
            after.review_requested_at = None;
            after.review_requested_by = None;
        } else {
            after.review_requested_at = Some(now);
            after.review_requested_by = Some(principal.user_id());
        }
        after.updated_at = now;
        let after = self.save_state(&transaction, &current, &after).await?;
        let mut before_summary = summary(&current);
        let mut after_summary = summary(&after);
        for (value, row) in [(&mut before_summary, &current), (&mut after_summary, &after)] {
            if let Some(object) = value.as_object_mut() {
                object.insert(
                    "review_requested_at".to_owned(),
                    serde_json::json!(row.review_requested_at),
                );
            }
        }
        let change =
            Change { before: Some(before_summary), after: Some(after_summary), published: None };
        let actor = Actor::from(principal);
        let subject = events::entry_subject(id);
        if withdraw {
            self.outbox
                .record(&transaction, actor, subject, &events::ReviewWithdrawn(change))
                .await?;
        } else {
            self.outbox
                .record(&transaction, actor, subject, &events::ReviewRequested(change))
                .await?;
        }
        transaction.commit().await?;
        tracing::info!(entry_id = %id, withdraw, "content review request changed");
        Ok(after)
    }
}
