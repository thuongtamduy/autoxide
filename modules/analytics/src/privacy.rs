//! Quyền riêng tư và hạn lưu:
//!
//! - job `analytics.cleanup` (mỗi ngày): xoá sự kiện thô quá `AUTOXIDE_ANALYTICS_RAW_RETENTION_DAYS`,
//!   số gộp quá `AUTOXIDE_ANALYTICS_RETENTION_DAYS`, khoá của các ngày đã qua (từ đó khoá phiên cũ
//!   không tính lại được), dấu "đã gộp" của ngày không còn sự kiện thô, bộ đếm tần suất cũ;
//! - rút đồng ý `analytics.personal` (event `compliance.consent_withdrawn`): gỡ `user_id` khỏi sự
//!   kiện của mọi người không còn đồng ý (số gộp không chứa người nên giữ nguyên);
//! - xoá tài khoản (`auth.user_deleted`): gỡ `user_id` của người đó;
//! - xuất dữ liệu cá nhân: sự kiện còn gắn với người đó.
//!
//! Mọi việc chạy lại bao nhiêu lần cũng ra cùng kết quả (bên nhận event chịu được nhận lại).

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::sync::Arc;

use autoxide_compliance::api::ConsentSubject;
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::job::{JobContext, JobHandler};
use autoxide_kernel::privacy::PersonalData;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::throttle::Throttle;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::entity::{daily, day_key, event, funnel_daily, rollup};
use crate::service::{AnalyticsService, PERSONAL_PURPOSE, day_of, now, personal_key};

pub(crate) const CLEANUP: &str = "analytics.cleanup";

/// Đã dọn những gì.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Cleaned {
    pub events: u64,
    pub daily: u64,
    pub keys: u64,
}

pub(crate) struct Cleanup(pub Arc<AnalyticsService>);

#[async_trait::async_trait]
impl JobHandler for Cleanup {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.0.cleanup(&job.database, now()).await.map(|_| ())
    }
}

impl AnalyticsService {
    /// Dọn `database` theo hạn lưu, coi `now` là bây giờ (test truyền thời điểm tương lai). Hạn
    /// lưu là của cả bản cài, nên quét cả database (index có `tenant_id` đứng đầu vẫn dùng được cho
    /// điều kiện ngày).
    pub(crate) async fn cleanup(
        &self,
        database: &Database,
        now: Timestamp,
    ) -> Result<Cleaned, Error> {
        let write = database.write();
        let raw_before =
            now - SignedDuration::from_hours(24 * (i64::from(self.config.raw_retention_days)));
        // tenant-scope: việc nền / kiểm tra quét mọi khách của database, chủ ý không lọc khách (rà soát bảo mật 2026-11)
        let events = event::Entity::delete_many()
            .filter(event::Column::OccurredAt.lt(raw_before))
            .exec(write)
            .await?
            .rows_affected;
        let kept_before =
            day_of(now - SignedDuration::from_hours(24 * (i64::from(self.config.retention_days))));
        // tenant-scope: việc nền / kiểm tra quét mọi khách của database, chủ ý không lọc khách (rà soát bảo mật 2026-11)
        let daily = daily::Entity::delete_many()
            .filter(daily::Column::Day.lt(kept_before.as_str()))
            .exec(write)
            .await?
            .rows_affected;
        // tenant-scope: việc nền / kiểm tra quét mọi khách của database, chủ ý không lọc khách (rà soát bảo mật 2026-11)
        funnel_daily::Entity::delete_many()
            .filter(funnel_daily::Column::Day.lt(kept_before.as_str()))
            .exec(write)
            .await?;
        let today = day_of(now);
        // tenant-scope: việc nền / kiểm tra quét mọi khách của database, chủ ý không lọc khách (rà soát bảo mật 2026-11)
        let keys = day_key::Entity::delete_many()
            .filter(day_key::Column::Day.lt(today.as_str()))
            .exec(write)
            .await?
            .rows_affected;
        // Dấu "đã gộp" chỉ cần cho ngày còn sự kiện thô.
        // tenant-scope: việc nền / kiểm tra quét mọi khách của database, chủ ý không lọc khách (rà soát bảo mật 2026-11)
        rollup::Entity::delete_many()
            .filter(rollup::Column::Day.lt(day_of(raw_before).as_str()))
            .exec(write)
            .await?;
        Throttle::new("analytics", 1, std::time::Duration::from_secs(60))?
            .purge(database, now - SignedDuration::from_hours(1))
            .await?;
        let cleaned = Cleaned { events, daily, keys };
        if cleaned != Cleaned::default() {
            tracing::info!(events, daily, keys, "analytics cleanup");
        }
        Ok(cleaned)
    }

    /// Gỡ `user_id` khỏi sự kiện của mọi người ở khách `tenant` không còn đồng ý theo dõi theo
    /// người; trả số sự kiện đã gỡ.
    pub(crate) async fn forget_withdrawn(&self, tenant: &TenantContext) -> Result<u64, Error> {
        let users: Vec<Uuid> = event::Entity::find()
            .select_only()
            .column(event::Column::UserId)
            .distinct()
            .filter(event::Column::TenantId.eq(tenant.id().as_uuid()))
            .filter(event::Column::UserId.is_not_null())
            .into_tuple::<Option<Uuid>>()
            .all(tenant.database().read())
            .await?
            .into_iter()
            .flatten()
            .collect();
        if users.is_empty() {
            return Ok(0);
        }
        let granted = match &self.consents {
            Some(ledger) => {
                let subjects: Vec<ConsentSubject> =
                    users.iter().copied().map(ConsentSubject::User).collect();
                ledger.granted_many(tenant.id(), &subjects, &personal_key()).await?
            }
            None => vec![false; users.len()],
        };
        let withdrawn: Vec<Uuid> = users
            .into_iter()
            .zip(granted)
            .filter_map(|(user, granted)| (!granted).then_some(user))
            .collect();
        let mut forgotten = 0;
        for chunk in withdrawn.chunks(500) {
            forgotten += self.forget_users(tenant, chunk).await?;
        }
        Ok(forgotten)
    }

    /// Gỡ `user_id` khỏi mọi sự kiện của `users` ở khách `tenant`.
    pub(crate) async fn forget_users(
        &self,
        tenant: &TenantContext,
        users: &[Uuid],
    ) -> Result<u64, Error> {
        if users.is_empty() {
            return Ok(0);
        }
        Ok(event::Entity::update_many()
            .col_expr(event::Column::UserId, Expr::value(Option::<Uuid>::None))
            .filter(event::Column::TenantId.eq(tenant.id().as_uuid()))
            .filter(event::Column::UserId.is_in(users.iter().copied()))
            .exec(tenant.database().write())
            .await?
            .rows_affected)
    }
}

/// Người dùng rút đồng ý (`compliance.consent_withdrawn`) hay bị xoá (`auth.user_deleted`).
pub(crate) struct Forget(pub Arc<AnalyticsService>);

#[async_trait::async_trait]
impl EventHandler for Forget {
    async fn handle(&self, delivered: &Delivered) -> Result<(), Error> {
        if let Some(user) = autoxide_kernel::privacy::deleted_user(delivered) {
            self.0.forget_users(&delivered.tenant, &[user]).await?;
            return Ok(());
        }
        // Rút đồng ý: payload không nói của ai, nên hỏi lại sổ đồng ý cho mọi người còn gắn sự kiện.
        // Mục đích khác thì bỏ qua; mục đích rỗng không có trong sổ (mục đích là bắt buộc).
        let purpose = delivered.payload.get("purpose").and_then(Value::as_str);
        if purpose == Some(PERSONAL_PURPOSE) {
            self.0.forget_withdrawn(&delivered.tenant).await?;
        }
        Ok(())
    }
}

// Dữ liệu cá nhân: sự kiện còn gắn với người dùng (chỉ có khi người đó đồng ý).
#[async_trait::async_trait]
impl PersonalData for AnalyticsService {
    async fn export(&self, tenant: &TenantContext, user_id: Uuid) -> Result<Value, Error> {
        let rows = event::Entity::find()
            .filter(event::Column::TenantId.eq(tenant.id().as_uuid()))
            .filter(event::Column::UserId.eq(user_id))
            .order_by_asc(event::Column::OccurredAt)
            .all(tenant.database().read())
            .await?;
        let events: Vec<Value> = rows
            .into_iter()
            .map(|row| {
                json!({
                    "occurred_at": row.occurred_at,
                    "type": row.kind,
                    "path": row.path,
                    "object": row.object,
                    "site": row.site_org_id,
                })
            })
            .collect();
        Ok(json!({ "events": events }))
    }
}
