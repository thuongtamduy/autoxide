//! Job dọn dẹp hằng ngày `forms.cleanup`: xoá bài spam quá `AUTOXIDE_FORMS_SPAM_RETENTION_DAYS`,
//! xoá bài quá thời hạn giữ của form (`retention_days`, mặc định theo
//! `AUTOXIDE_FORMS_RETENTION_DAYS`; 0 là giữ mãi), xoá dấu vết IP quá
//! `AUTOXIDE_FORMS_IP_RETENTION_DAYS`, xoá bài "mồ côi" (form của nó không còn — lưới an toàn,
//! vì không có foreign key), xoá bản xuất CSV quá `AUTOXIDE_FORMS_EXPORT_KEEP_HOURS` (file
//! trước, dòng sau), và dọn nonce, bộ đếm đã hết hạn. Chạy lại bao nhiêu lần cũng ra
//! cùng kết quả.

use std::sync::Arc;
use std::time::Duration;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::{JobContext, JobHandler};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::throttle::Throttle;
use chrono::{DateTime, Utc};
use sea_orm::sea_query::{Expr, Query};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::json;

use crate::entity::{form, submission};
use crate::events::{self, Change};
use crate::service::{FormsService, now};
use crate::submissions::inbox::SubmissionStatus;

pub(crate) const CLEANUP: &str = "forms.cleanup";

/// Đã dọn những gì.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Cleaned {
    pub spam: u64,
    pub expired: u64,
    pub fingerprints: u64,
    /// Bài của form đã bị xoá.
    pub orphans: u64,
    /// Bản xuất CSV quá hạn giữ (file và dòng).
    pub exports: u64,
}

pub(crate) struct Cleanup(pub Arc<FormsService>);

#[async_trait::async_trait]
impl JobHandler for Cleanup {
    async fn run(&self, _: &JobContext) -> Result<(), Error> {
        self.0.cleanup(now()).await.map(|_| ())
    }
}

impl FormsService {
    /// Dọn theo thời hạn, coi `now` là bây giờ (test truyền thời điểm tương lai).
    pub(crate) async fn cleanup(&self, now: DateTime<Utc>) -> Result<Cleaned, Error> {
        let mut cleaned = Cleaned::default();
        let spam_before = now - chrono::Duration::days(i64::from(self.config.spam_retention_days));
        let forms = form::Entity::find().all(self.database.read()).await?;
        for form in &forms {
            cleaned.spam +=
                self.expire(form, spam_before, Some(SubmissionStatus::Spam), "spam").await?;
            let days = form
                .retention_days
                .and_then(|days| u32::try_from(days).ok())
                .unwrap_or(self.config.retention_days);
            if days > 0 {
                let before = now - chrono::Duration::days(i64::from(days));
                cleaned.expired += self.expire(form, before, None, "retention").await?;
            }
        }
        let ip_before = now - chrono::Duration::days(i64::from(self.config.ip_retention_days));
        let write = self.database.write();
        cleaned.fingerprints = submission::Entity::update_many()
            .col_expr(submission::Column::IpFingerprint, Expr::value(Option::<String>::None))
            .filter(submission::Column::IpFingerprint.is_not_null())
            .filter(submission::Column::CreatedAt.lt(ip_before))
            .exec(write)
            .await?
            .rows_affected;
        // Bài mà form không còn (không có foreign key; nhận bài đã khoá dòng form nên không
        // còn đường tạo bài mồ côi, đây là lưới an toàn cho dữ liệu cũ): xoá, dữ liệu cá nhân
        // không ai đọc được thì không giữ.
        cleaned.orphans = submission::Entity::delete_many()
            .filter(submission::Column::FormId.not_in_subquery(
                Query::select().column(form::Column::Id).from(form::Entity).to_owned(),
            ))
            .exec(write)
            .await?
            .rows_affected;
        let expired = self.expired_exports(now).await?;
        cleaned.exports = self.purge_exports(expired).await?;
        self.guard.purge_nonces(write, "forms").await?;
        Throttle::new("forms", 1, Duration::from_secs(3600))?
            .purge(&self.database, now - chrono::Duration::days(1))
            .await?;
        if cleaned != Cleaned::default() {
            tracing::info!(
                spam = cleaned.spam,
                expired = cleaned.expired,
                fingerprints = cleaned.fingerprints,
                orphans = cleaned.orphans,
                exports = cleaned.exports,
                "forms cleanup"
            );
        }
        Ok(cleaned)
    }

    /// Xoá bài của `form` gửi trước `before` (chỉ trạng thái `status`, nếu có), kèm một event.
    async fn expire(
        &self,
        form: &form::Model,
        before: DateTime<Utc>,
        status: Option<SubmissionStatus>,
        reason: &str,
    ) -> Result<u64, Error> {
        let transaction = self.database.begin_write().await?;
        let mut delete = submission::Entity::delete_many()
            .filter(submission::Column::FormId.eq(form.id))
            .filter(submission::Column::CreatedAt.lt(before));
        if let Some(status) = status {
            delete = delete.filter(submission::Column::Status.eq(status.as_str()));
        }
        let count = delete.exec(&transaction).await?.rows_affected;
        if count > 0 {
            let change = Change {
                before: Some(
                    json!({"form_id": form.id, "form_key": form.key, "count": count, "reason": reason}),
                ),
                after: None,
            };
            self.outbox
                .record(
                    &transaction,
                    &TenantContext::legacy_default(&self.database),
                    Actor::system(),
                    events::form_subject(form.id),
                    &events::SubmissionsExpired(change),
                )
                .await?;
        }
        transaction.commit().await?;
        Ok(count)
    }
}
