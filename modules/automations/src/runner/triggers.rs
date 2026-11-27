//! Kích hoạt theo event: nghe MỌI event (`Subscription::ALL`), tìm luật đang bật của khách nghe đúng
//! event đó (một câu trên index `(tenant, trigger_type, trigger_ref, status)`), tính điều kiện, rồi
//! ghi lần chạy + job. Không chạy hành động trong bộ nhận event.
//!
//! Chống vòng lặp: event do hành động của một lần chạy sinh ra mang `AccessPath::Automation(run_id)`.
//! Cùng luật thì không kích hoạt lại; luật khác thì độ sâu chuỗi tăng một, quá
//! [`MAX_CHAIN_DEPTH`](crate::config::MAX_CHAIN_DEPTH) thì ghi lần chạy `skipped`
//! (`automations.loop_detected`) thay vì chạy.
//!
//! Phạm vi tổ chức (L-8 của rà soát bảo mật 2026-11, D-20261125-event-org): luật của nút gốc nhận event của cả
//! khách; luật của một nút con chỉ nhận event mà [`Delivered::org_id`] nằm trong nhánh của nút đó. Event không
//! nêu nút (của cả khách, hay module chưa ghi nút) chỉ tới luật của nút gốc — như luồng event (SSE) chỉ cho
//! người có quyền cả khách thấy event không rõ nhánh.

use std::sync::Arc;

use autoxide_kernel::access::AccessPath;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext, root_of};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};

use super::Start;
use crate::config::MAX_CHAIN_DEPTH;
use crate::definition::Condition;
use crate::entity::{rule, run};
use crate::service::Automations;
use crate::service::rules::{ACTIVE, parse_json};

/// Event của module `auth` khi một API key bị thu hồi (chủ đề `auth.api_key:<id>`).
const API_KEY_REVOKED: &str = "auth.api_key_revoked";
/// Event của module `auth` khi một grant OAuth (client MCP) bị thu hồi (chủ đề `auth.oauth_grant:<id>`).
const OAUTH_GRANT_REVOKED: &str = "auth.oauth_revoked";

/// Bộ nhận mọi event.
pub(crate) struct EventTrigger(pub Arc<Automations>);

#[async_trait::async_trait]
impl EventHandler for EventTrigger {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let tenant = &event.tenant;
        // API key bị thu hồi: luật nó đã tạo / bật tạm dừng (D-20261121-automation-credential).
        if event.name == API_KEY_REVOKED
            && let Some(key) = event
                .subject
                .as_deref()
                .and_then(|subject| subject.strip_prefix("auth.api_key:"))
                .and_then(|id| id.parse::<uuid::Uuid>().ok())
        {
            self.0.credential_revoked(tenant, key).await?;
        }
        // Grant OAuth bị thu hồi: luật client đó đã tạo / bật tạm dừng như với API key (D-20261126-mcp-oauth).
        if event.name == OAUTH_GRANT_REVOKED
            && let Some(grant) = event
                .subject
                .as_deref()
                .and_then(|subject| subject.strip_prefix("auth.oauth_grant:"))
                .and_then(|id| id.parse::<uuid::Uuid>().ok())
        {
            self.0.credential_revoked(tenant, grant).await?;
        }
        // Mọi cửa vào tài khoản `run_as` vừa bị cắt (khoá, lấy lại bằng mail, khoá hẳn, xoá — có thể vì kẻ lạ
        // đã chiếm nó): luật của nó tạm dừng tới khi một người có quyền xem lại và bật lại
        // (D-20261126-automation-owner-revoked).
        if let Some(user) = autoxide_kernel::accounts::revoked_user(event) {
            self.0.owner_revoked(tenant, user).await?;
        }
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
        let event_org = event.org_id();
        let root = root_of(tenant.id());
        for rule in rules {
            if !self.0.reaches(tenant, root, OrgId::from_uuid(rule.org_id), event_org).await? {
                continue;
            }
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

impl Automations {
    // Event ở nút `event_org` có thuộc phạm vi của luật ở nút `rule_org` không (L-8). Hỏi cây tổ chức
    // (module `organizations`) chỉ khi cần; không có cây thì chỉ nút trùng mới tính.
    async fn reaches(
        &self,
        tenant: &TenantContext,
        root: OrgId,
        rule_org: OrgId,
        event_org: Option<OrgId>,
    ) -> Result<bool, Error> {
        if rule_org == root {
            return Ok(true);
        }
        let Some(event_org) = event_org else { return Ok(false) };
        if event_org == rule_org {
            return Ok(true);
        }
        match &self.orgs {
            Some(orgs) => orgs.is_within(tenant, event_org, rule_org).await,
            None => Ok(false),
        }
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
