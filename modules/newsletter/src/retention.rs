//! Job dọn dẹp hằng ngày `newsletter.cleanup`: xoá đăng ký chưa xác nhận quá
//! `AUTOXIDE_NEWSLETTER_UNCONFIRMED_DAYS`, xoá người đăng ký không còn đăng ký và lượt gửi nào
//! (người chỉ xin đăng ký mà không xác nhận), dọn token, nonce, bộ đếm đã hết hạn. Chạy lại bao
//! nhiêu lần cũng ra cùng kết quả.

use std::sync::Arc;
use std::time::Duration;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::{JobContext, JobHandler};
use autoxide_kernel::throttle::Throttle;
use chrono::{DateTime, Utc};
use sea_orm::sea_query::Query;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::json;

use crate::entity::{delivery, subscriber, subscription};
use crate::events::{self, Change};
use crate::service::{MODULE, NewsletterService, now};
use crate::subscribers::SubscriptionStatus;

pub(crate) const CLEANUP: &str = "newsletter.cleanup";

/// Đã dọn những gì.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Cleaned {
    pub subscriptions: u64,
    pub subscribers: u64,
}

pub(crate) struct Cleanup(pub Arc<NewsletterService>);

#[async_trait::async_trait]
impl JobHandler for Cleanup {
    async fn run(&self, _: &JobContext) -> Result<(), Error> {
        self.0.cleanup(now()).await.map(|_| ())
    }
}

impl NewsletterService {
    /// Dọn theo thời hạn, coi `now` là bây giờ (test truyền thời điểm tương lai).
    pub(crate) async fn cleanup(&self, now: DateTime<Utc>) -> Result<Cleaned, Error> {
        let before = now - chrono::Duration::days(i64::from(self.config.unconfirmed_days));
        let transaction = self.database.begin_write().await?;
        let subscriptions = subscription::Entity::delete_many()
            .filter(subscription::Column::Status.eq(SubscriptionStatus::Unconfirmed.as_str()))
            .filter(subscription::Column::RequestedAt.lt(before))
            .exec(&transaction)
            .await?
            .rows_affected;
        // Người không còn đăng ký nào, chưa từng được gửi gì, tạo trước mốc.
        let subscribers = subscriber::Entity::delete_many()
            .filter(subscriber::Column::CreatedAt.lt(before))
            .filter(
                subscriber::Column::Id.not_in_subquery(
                    Query::select()
                        .column(subscription::Column::SubscriberId)
                        .from(subscription::Entity)
                        .to_owned(),
                ),
            )
            .filter(
                subscriber::Column::Id.not_in_subquery(
                    Query::select()
                        .column(delivery::Column::SubscriberId)
                        .from(delivery::Entity)
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
                .record(&transaction, Actor::system(), None, &events::SubscriptionsExpired(change))
                .await?;
        }
        transaction.commit().await?;
        let write = self.database.write();
        self.tokens.purge(write, now - chrono::Duration::days(1)).await?;
        self.guard.purge_nonces(write, MODULE).await?;
        Throttle::new(self.database.clone(), MODULE, 1, Duration::from_secs(3600))?
            .purge(now - chrono::Duration::days(2))
            .await?;
        let cleaned = Cleaned { subscriptions, subscribers };
        if cleaned != Cleaned::default() {
            tracing::info!(subscriptions, subscribers, "newsletter cleanup");
        }
        Ok(cleaned)
    }
}
