//! Job dọn dẹp hằng ngày `forms.cleanup`: xoá bài spam quá `AUTOXIDE_FORMS_SPAM_RETENTION_DAYS`,
//! xoá bài quá thời hạn giữ của form (`retention_days`, mặc định theo
//! `AUTOXIDE_FORMS_RETENTION_DAYS`; 0 là giữ mãi), xoá dấu vết IP quá
//! `AUTOXIDE_FORMS_IP_RETENTION_DAYS`, xoá bài "mồ côi" (form của nó không còn — lưới an toàn,
//! vì không có foreign key), xoá bản xuất CSV quá `AUTOXIDE_FORMS_EXPORT_KEEP_HOURS` (file
//! trước, dòng sau), và dọn nonce, bộ đếm đã hết hạn. Chạy lại bao nhiêu lần cũng ra
//! cùng kết quả.
//!
//! Job theo lịch chạy một lần cho mỗi database (bước 33): việc gắn với form (thời hạn giữ, bản
//! xuất, event) chạy theo từng khách có dữ liệu trên database đó; xoá dấu vết IP, bài mồ côi,
//! nonce và bộ đếm quét cả database.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::{JobContext, JobHandler};
use autoxide_kernel::tenancy::{DatabaseRegistry, ScopeExt, TenantContext, TenantId};
use autoxide_kernel::throttle::Throttle;
use chrono::{DateTime, Utc};
use sea_orm::sea_query::{Expr, Query};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
use serde_json::json;
use uuid::Uuid;

use crate::entity::{export, form, submission};
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

pub(crate) struct Cleanup {
    pub service: Arc<FormsService>,
    /// Sổ database: tra ngữ cảnh của từng khách có dữ liệu trên database của job.
    pub databases: DatabaseRegistry,
}

#[async_trait::async_trait]
impl JobHandler for Cleanup {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.service.cleanup(&self.databases, &job.database, now()).await.map(|_| ())
    }
}

impl FormsService {
    /// Dọn `database` theo thời hạn, coi `now` là bây giờ (test truyền thời điểm tương lai).
    pub(crate) async fn cleanup(
        &self,
        databases: &DatabaseRegistry,
        database: &Database,
        now: DateTime<Utc>,
    ) -> Result<Cleaned, Error> {
        let mut cleaned = Cleaned::default();
        // Khách có form hay bản xuất trên database này (quét cả database, qua index có
        // `tenant_id` đứng đầu).
        let mut tenants: BTreeSet<Uuid> = form::Entity::find()
            .select_only()
            .column(form::Column::TenantId)
            .distinct()
            .into_tuple::<Uuid>()
            .all(database.read())
            .await?
            .into_iter()
            .collect();
        tenants.extend(
            export::Entity::find()
                .select_only()
                .column(export::Column::TenantId)
                .distinct()
                .into_tuple::<Uuid>()
                .all(database.read())
                .await?,
        );
        for id in tenants {
            match databases.tenant(TenantId::from_uuid(id)).await {
                Ok(tenant) => self.cleanup_tenant(&tenant, now, &mut cleaned).await?,
                // Khách đã bị xoá khỏi sổ: dữ liệu còn lại do việc xoá khách lo, không đoán ở đây.
                Err(error) => {
                    tracing::warn!(tenant_id = %id, error = %error, "forms cleanup skipped a tenant");
                }
            }
        }
        let ip_before = now - chrono::Duration::days(i64::from(self.config.ip_retention_days));
        let write = database.write();
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
        self.guard.purge_nonces(write, "forms").await?;
        Throttle::new("forms", 1, Duration::from_secs(3600))?
            .purge(database, now - chrono::Duration::days(1))
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

    /// Phần dọn của khách `tenant`: bài quá hạn giữ của từng form, bản xuất quá hạn.
    async fn cleanup_tenant(
        &self,
        tenant: &TenantContext,
        now: DateTime<Utc>,
        cleaned: &mut Cleaned,
    ) -> Result<(), Error> {
        let spam_before = now - chrono::Duration::days(i64::from(self.config.spam_retention_days));
        let forms = form::Entity::find().scoped(tenant).all(tenant.database().read()).await?;
        for form in &forms {
            cleaned.spam += self
                .expire(tenant, form, spam_before, Some(SubmissionStatus::Spam), "spam")
                .await?;
            let days = form
                .retention_days
                .and_then(|days| u32::try_from(days).ok())
                .unwrap_or(self.config.retention_days);
            if days > 0 {
                let before = now - chrono::Duration::days(i64::from(days));
                cleaned.expired += self.expire(tenant, form, before, None, "retention").await?;
            }
        }
        let expired = self.expired_exports(tenant, now).await?;
        cleaned.exports += self.purge_exports(tenant, expired).await?;
        Ok(())
    }

    /// Xoá bài của `form` gửi trước `before` (chỉ trạng thái `status`, nếu có), kèm một event.
    async fn expire(
        &self,
        tenant: &TenantContext,
        form: &form::Model,
        before: DateTime<Utc>,
        status: Option<SubmissionStatus>,
        reason: &str,
    ) -> Result<u64, Error> {
        let transaction = tenant.database().begin_write().await?;
        let mut delete = submission::Entity::delete_many()
            .scoped(tenant)
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
                    tenant,
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
