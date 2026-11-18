//! Lịch của module, chạy trên TỪNG database:
//!
//! - `automations.tick` (mỗi `AUTOXIDE_AUTOMATIONS_TICK_SECONDS`): luật đang bật tới hạn chạy theo
//!   lịch (`next_run_at <= bây giờ`, index quét `(status, next_run_at)`). Mỗi luật: dời `next_run_at`
//!   sang mốc kế bằng một câu `UPDATE … WHERE next_run_at = <mốc cũ>` (hai worker không cùng lấy),
//!   rồi ghi lần chạy với khoá `schedule:<mốc>` — chạy lại cũng không thành hai lần.
//! - `automations.prune` (mỗi ngày): xoá nhật ký lần chạy đã xong quá
//!   `AUTOXIDE_AUTOMATIONS_RUN_RETENTION_DAYS` ngày, cùng khoá chống chạy trùng của chúng.

use std::sync::Arc;

use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::job::{JobContext, JobHandler};
use autoxide_kernel::tenancy::{TenantContext, TenantId};
use chrono::{DateTime, Duration, Utc};
use chrono_tz::Tz;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use serde_json::json;

use super::{FAILED, SKIPPED, SUCCEEDED, Start};
use crate::definition::Trigger;
use crate::entity::{rule, run, run_key};
use crate::service::rules::{ACTIVE, Rule, parse_json};
use crate::service::{Automations, now};

/// Loại job (và tên lịch) tìm luật tới hạn.
pub(crate) const TICK: &str = "automations.tick";
/// Loại job (và tên lịch) dọn nhật ký cũ.
pub(crate) const PRUNE: &str = "automations.prune";
/// Số luật tới hạn xử lý trong một lần quét (còn nữa thì lần sau).
const TICK_BATCH: u64 = 200;

pub(crate) struct TickJob(pub Arc<Automations>);

#[async_trait::async_trait]
impl JobHandler for TickJob {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.0.tick(&job.database).await
    }
}

pub(crate) struct PruneJob(pub Arc<Automations>);

#[async_trait::async_trait]
impl JobHandler for PruneJob {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.0.prune(&job.database).await
    }
}

impl Automations {
    /// Chạy các luật tới hạn của `database` (mọi khách ở database đó).
    pub(crate) async fn tick(&self, database: &Database) -> Result<(), Error> {
        let now = now();
        let due = rule::Entity::find()
            .filter(rule::Column::Status.eq(ACTIVE))
            .filter(rule::Column::NextRunAt.lte(now))
            .order_by_asc(rule::Column::NextRunAt)
            .limit(TICK_BATCH)
            .all(database.read())
            .await?;
        for rule in due {
            let tenant = match self.databases.tenant(TenantId::from_uuid(rule.tenant_id)).await {
                Ok(tenant) => tenant,
                Err(error) => {
                    tracing::info!(rule_id = %rule.id, %error, "tenant not available; the rule waits");
                    continue;
                }
            };
            if let Err(error) = self.fire(&tenant, &rule, now).await {
                // Một luật lỗi không chặn các luật khác; lần quét sau thử lại (mốc chưa dời).
                tracing::warn!(rule_id = %rule.id, %error, "a scheduled automation rule failed to start");
            }
        }
        Ok(())
    }

    /// Một luật tới hạn: dời mốc, rồi kích hoạt theo loại.
    async fn fire(
        &self,
        tenant: &TenantContext,
        rule: &Rule,
        now: DateTime<Utc>,
    ) -> Result<(), Error> {
        let Some(slot) = rule.next_run_at else { return Ok(()) };
        let mut errors = Vec::new();
        let Some((trigger, _)) = Trigger::parse(&parse_json(&rule.trigger), "trigger", &mut errors)
        else {
            return Ok(());
        };
        let zone = rule.timezone.parse::<Tz>().unwrap_or(Tz::UTC);
        let next = trigger.next_run(now, zone);
        // Chỉ một worker dời được mốc này.
        let moved = rule::Entity::update_many()
            .col_expr(rule::Column::NextRunAt, Expr::value(next))
            .filter(rule::Column::Id.eq(rule.id))
            .filter(rule::Column::TenantId.eq(rule.tenant_id))
            .filter(rule::Column::NextRunAt.eq(slot))
            .exec(tenant.database().write())
            .await?;
        if moved.rows_affected == 0 {
            return Ok(());
        }
        match trigger {
            Trigger::Schedule(_) => {
                let start = Start {
                    trigger_key: format!("schedule:{}", slot.to_rfc3339()),
                    event: None,
                    context: json!({
                        "event": {"scheduled_at": slot},
                        "trigger": {"type": "schedule", "scheduled_at": slot},
                    }),
                    depth: 1,
                    triggered_by: None,
                    skip: None,
                };
                if super::triggers::holds(rule, &start.context) {
                    self.start_run(tenant, rule, start).await?;
                }
                Ok(())
            }
            Trigger::Metric(metric) => self.check_metric(tenant, rule, &metric, slot, zone).await,
            Trigger::Event { .. } | Trigger::Manual => Ok(()),
        }
    }

    /// Xoá nhật ký lần chạy đã xong quá hạn lưu của `database`.
    pub(crate) async fn prune(&self, database: &Database) -> Result<(), Error> {
        let before = now() - Duration::days(self.config.run_retention_days);
        let deleted = run::Entity::delete_many()
            .filter(run::Column::CreatedAt.lt(before))
            .filter(run::Column::Status.is_in([SUCCEEDED, FAILED, SKIPPED]))
            .exec(database.write())
            .await?;
        run_key::Entity::delete_many()
            .filter(run_key::Column::CreatedAt.lt(before))
            .exec(database.write())
            .await?;
        self.prune_metrics(database).await?;
        tracing::info!(runs = deleted.rows_affected, "old automation runs removed");
        Ok(())
    }
}
