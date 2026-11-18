//! Kích hoạt theo event: nghe MỌI event (`Subscription::ALL`), tìm luật đang bật của khách nghe đúng
//! event đó (một câu trên index `(tenant, trigger_type, trigger_ref, status)`), tính điều kiện, rồi
//! ghi lần chạy + job. Không chạy hành động trong bộ nhận event.
//!
//! Chống vòng lặp: event do hành động của một lần chạy sinh ra mang `AccessPath::Automation(run_id)`.
//! Cùng luật thì không kích hoạt lại; luật khác thì độ sâu chuỗi tăng một, quá
//! [`MAX_CHAIN_DEPTH`](crate::config::MAX_CHAIN_DEPTH) thì ghi lần chạy `skipped`
//! (`automations.loop_detected`) thay vì chạy.

use std::sync::Arc;

use autoxide_kernel::access::AccessPath;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::tenancy::ScopeExt;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};

use super::Start;
use crate::config::MAX_CHAIN_DEPTH;
use crate::definition::Condition;
use crate::entity::{rule, run};
use crate::service::Automations;
use crate::service::rules::{ACTIVE, parse_json};

/// Bộ nhận mọi event.
pub(crate) struct EventTrigger(pub Arc<Automations>);

#[async_trait::async_trait]
impl EventHandler for EventTrigger {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let tenant = &event.tenant;
        let rules = rule::Entity::find()
            .filter(rule::Column::TriggerType.eq("event"))
            .filter(rule::Column::TriggerRef.eq(event.name.as_str()))
            .filter(rule::Column::Status.eq(ACTIVE))
            .scoped(tenant)
            .all(tenant.database().read())
            .await?;
        if rules.is_empty() {
            return Ok(());
        }
        // Lần chạy đã gây ra event này (nếu có).
        let parent = match event.actor.access {
            AccessPath::Automation(run_id) => {
                run::Entity::find_by_id(run_id).scoped(tenant).one(tenant.database().read()).await?
            }
            _ => None,
        };
        let context = json!({
            "event": event.payload,
            "trigger": {
                "type": "event",
                "name": event.name,
                "event_id": event.id,
                "subject": event.subject,
                "occurred_at": event.occurred_at,
                "actor_user_id": event.actor.user_id,
            },
        });
        for rule in rules {
            let depth = parent.as_ref().map_or(1, |parent| parent.depth + 1);
            let skip = match &parent {
                Some(parent) if parent.rule_id == rule.id => Some((
                    "automations.loop_detected",
                    "The event came from a run of this same rule; a rule never starts itself again.",
                )),
                Some(_) if depth > MAX_CHAIN_DEPTH => Some((
                    "automations.loop_detected",
                    "Too many rules in a chain (a rule's action started another rule, and so on).",
                )),
                _ => None,
            };
            if skip.is_none() && !holds(&rule, &context) {
                continue;
            }
            let start = Start {
                trigger_key: format!("event:{}", event.id),
                event: Some((event.id, event.name.clone())),
                context: context.clone(),
                depth,
                triggered_by: None,
                skip,
            };
            self.0.start_run(tenant, &rule, start).await?;
        }
        Ok(())
    }
}

/// Điều kiện của luật (lưu đã kiểm) có đúng với `context` không. Thân lưu hỏng thì không chạy.
pub(crate) fn holds(rule: &rule::Model, context: &Value) -> bool {
    let mut errors = Vec::new();
    let condition = Condition::parse(&parse_json(&rule.conditions), "conditions", &mut errors);
    if !errors.is_empty() {
        tracing::warn!(rule_id = %rule.id, "an automation rule has invalid stored conditions");
        return false;
    }
    Automations::matches(condition.as_ref(), context)
}
