//! Job dọn dẹp hằng ngày `newsletter.cleanup`: xoá đăng ký chưa xác nhận quá
//! `AUTOXIDE_NEWSLETTER_UNCONFIRMED_DAYS`, xoá người đăng ký không còn đăng ký và lượt gửi nào
//! (người chỉ xin đăng ký mà không xác nhận), dọn token, nonce, bộ đếm đã hết hạn. Chạy lại bao
//! nhiêu lần cũng ra cùng kết quả.
//!
//! Job theo lịch chạy một lần cho mỗi database (bước 33): việc xoá đăng ký và người đăng ký (kèm
//! event) chạy theo từng khách có dữ liệu trên database đó; token, nonce và bộ đếm quét cả
//! database.

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
use sea_orm::sea_query::Query;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
use serde_json::json;
use uuid::Uuid;

use crate::audience::subscribers::SubscriptionStatus;
use crate::entity::{delivery, subscriber, subscription};
use crate::events::{self, Change};
use crate::service::{MODULE, NewsletterService, now};

pub(crate) const CLEANUP: &str = "newsletter.cleanup";

/// Đã dọn những gì.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Cleaned {
    pub subscriptions: u64,
    pub subscribers: u64,
}

pub(crate) struct Cleanup {
    pub service: Arc<NewsletterService>,
    /// Sổ database: tra ngữ cảnh của từng khách có dữ liệu trên database của job.
    pub databases: DatabaseRegistry,
}

#[async_trait::async_trait]
impl JobHandler for Cleanup {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.service.cleanup(&self.databases, &job.database, now()).await.map(|_| ())
    }
}

impl NewsletterService {
    /// Dọn `database` theo thời hạn, coi `now` là bây giờ (test truyền thời điểm tương lai).
    pub(crate) async fn cleanup(
        &self,
        databases: &DatabaseRegistry,
        database: &Database,
        now: DateTime<Utc>,
    ) -> Result<Cleaned, Error> {
        let mut cleaned = Cleaned::default();
        // Khách có người đăng ký hay đăng ký trên database này (quét cả database, qua index có
        // `tenant_id` đứng đầu).
        let mut tenants: BTreeSet<Uuid> = subscriber::Entity::find()
            .select_only()
            .column(subscriber::Column::TenantId)
            .distinct()
            .into_tuple::<Uuid>()
            .all(database.read())
            .await?
            .into_iter()
            .collect();
        tenants.extend(
            subscription::Entity::find()
                .select_only()
                .column(subscription::Column::TenantId)
                .distinct()
                .into_tuple::<Uuid>()
                .all(database.read())
                .await?,
        );
        for id in tenants {
            match databases.tenant(TenantId::from_uuid(id)).await {
                Ok(tenant) => {
                    let part = self.cleanup_tenant(&tenant, now).await?;
                    cleaned.subscriptions += part.subscriptions;
                    cleaned.subscribers += part.subscribers;
                }
                // Khách đã bị xoá khỏi sổ: dữ liệu còn lại do việc xoá khách lo, không đoán ở đây.
                Err(error) => {
                    tracing::warn!(tenant_id = %id, error = %error, "newsletter cleanup skipped a tenant");
                }
            }
        }
        let write = database.write();
        self.tokens.purge(write, now - chrono::Duration::days(1)).await?;
        self.guard.purge_nonces(write, MODULE).await?;
        Throttle::new(MODULE, 1, Duration::from_secs(3600))?
            .purge(database, now - chrono::Duration::days(2))
            .await?;
        if cleaned != Cleaned::default() {
            tracing::info!(
                subscriptions = cleaned.subscriptions,
                subscribers = cleaned.subscribers,
                "newsletter cleanup"
            );
        }
        Ok(cleaned)
    }

    /// Phần dọn của khách `tenant`: đăng ký chưa xác nhận quá hạn, người không còn gì, kèm event.
    async fn cleanup_tenant(
        &self,
        tenant: &TenantContext,
        now: DateTime<Utc>,
    ) -> Result<Cleaned, Error> {
        let before = now - chrono::Duration::days(i64::from(self.config.unconfirmed_days));
        let tenant_id = tenant.id().as_uuid();
        let transaction = tenant.database().begin_write().await?;
        let subscriptions = subscription::Entity::delete_many()
            .scoped(tenant)
            .filter(subscription::Column::Status.eq(SubscriptionStatus::Unconfirmed.as_str()))
            .filter(subscription::Column::RequestedAt.lt(before))
            .exec(&transaction)
            .await?
            .rows_affected;
        // Người không còn đăng ký nào, chưa từng được gửi gì, tạo trước mốc. Câu con cũng chỉ
        // xét dòng của khách này.
        let subscribers = subscriber::Entity::delete_many()
            .scoped(tenant)
            .filter(subscriber::Column::CreatedAt.lt(before))
            .filter(
                subscriber::Column::Id.not_in_subquery(
                    Query::select()
                        .column(subscription::Column::SubscriberId)
                        .from(subscription::Entity)
                        .and_where(subscription::Column::TenantId.eq(tenant_id))
                        .to_owned(),
                ),
            )
            .filter(
                subscriber::Column::Id.not_in_subquery(
                    Query::select()
                        .column(delivery::Column::SubscriberId)
                        .from(delivery::Entity)
                        .and_where(delivery::Column::TenantId.eq(tenant_id))
                        .to_owned(),
                ),
            )
            .exec(&transaction)
            .await?
            .rows_affected;
        if subscriptions > 0 || subscribers > 0 {
            let change = Change {
                before: Some(json!({"subscriptions": subscriptions, "subscribers": subscribers})),
                after: None,
            };
            self.outbox
                .record(
                    &transaction,
                    tenant,
                    Actor::system(),
                    None,
                    &events::SubscriptionsExpired(change),
                )
                .await?;
        }
        transaction.commit().await?;
        Ok(Cleaned { subscriptions, subscribers })
    }
}
