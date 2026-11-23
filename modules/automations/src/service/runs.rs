//! Lần chạy: xem nhật ký, chạy tay, chạy lại lần hỏng, duyệt / từ chối bước rủi ro.

use autoxide_kernel::access::{ActorKind, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::job::EnqueueOptions;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::time::Timestamp;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};
use uuid::Uuid;

use super::rules::{ACTIVE, target};
use super::{Automations, human_required, now, run_not_found};
use crate::definition::Action;
use crate::entity::run::{self, Column, Entity};
use crate::permissions::{AUTOMATIONS_READ, AUTOMATIONS_RUN};
use crate::runner::{AWAITING_APPROVAL, FAILED, QUEUED, RunJob, Start, StepLog};
use crate::service::rules::parse_json;

pub(crate) type Run = run::Model;

/// Bộ lọc danh sách lần chạy.
#[derive(Debug, Clone, Default)]
pub(crate) struct RunFilter {
    pub rule_id: Option<Uuid>,
    pub status: Option<String>,
}

impl Automations {
    /// Một trang lần chạy trong phạm vi `automations.read` của người gọi, mới trước (mặc định).
    pub(crate) async fn list_runs(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        filter: &RunFilter,
        page: &PageRequest,
    ) -> Result<Page<Run>, Error> {
        let scope = principal.scope_condition(&AUTOMATIONS_READ, Column::OrgId, None)?;
        let mut select = Entity::find().filter(scope).scoped(tenant);
        if let Some(rule_id) = filter.rule_id {
            select = select.filter(Column::RuleId.eq(rule_id));
        }
        if let Some(status) = &filter.status {
            select = select.filter(Column::Status.eq(status.as_str()));
        }
        fetch_page(select, Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    /// Lần chạy `id`; ngoài phạm vi `automations.read` báo như không có.
    pub(crate) async fn get_run(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Run, Error> {
        let read = tenant.database().read();
        let run = Entity::find_by_id(id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(|| run_not_found(id))?;
        let target = autoxide_kernel::access::OrgTarget::new(
            autoxide_kernel::tenancy::OrgId::from_uuid(run.org_id),
            None,
        );
        principal.require_at(read, &AUTOMATIONS_READ, target).await.map_err(|error| {
            if error.code() == "org_out_of_scope" { run_not_found(id) } else { error }
        })?;
        Ok(run)
    }

    /// Chạy tay luật `rule_id` với dữ liệu `input` (thành `event` của mẫu). Luật phải đang bật.
    pub(crate) async fn run_rule(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        rule_id: Uuid,
        input: Value,
    ) -> Result<Run, Error> {
        principal.require(&AUTOMATIONS_RUN)?;
        let rule = self.get_rule(tenant, principal, rule_id).await?;
        principal.require_at(tenant.database().read(), &AUTOMATIONS_RUN, target(&rule)).await?;
        if rule.status != ACTIVE {
            return Err(rule_paused(&rule.status));
        }
        let id = Uuid::now_v7();
        let context = json!({
            "event": input,
            "trigger": {"type": "manual", "user_id": principal.user_id(), "occurred_at": now()},
        });
        let start = Start {
            trigger_key: format!("manual:{id}"),
            event: None,
            context,
            depth: 1,
            triggered_by: Some(principal.user_id()),
            skip: None,
        };
        self.start_run(tenant, &rule, start).await?.ok_or_else(|| rule_paused("paused"))
    }

    /// Chạy lại lần chạy hỏng `id` từ bước hỏng; các bước đã xong không làm lại.
    pub(crate) async fn retry_run(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Run, Error> {
        let run = self.runnable(tenant, principal, id).await?;
        if run.status != FAILED {
            return Err(run_state(&run.status, "failed"));
        }
        self.requeue(
            tenant,
            &run,
            |update| {
                update
                    .col_expr(Column::ErrorCode, Expr::value(Option::<String>::None))
                    .col_expr(Column::ErrorMessage, Expr::value(Option::<String>::None))
                    .col_expr(Column::FinishedAt, Expr::value(Option::<Timestamp>::None))
                    .col_expr(Column::TriggeredBy, Expr::value(principal.user_id()))
            },
            FAILED,
        )
        .await
    }

    /// Duyệt bước rủi ro đang chờ của lần chạy `id`: người duyệt (không phải AI agent) cần
    /// `automations.run` ở nút của luật và permission của chính tool đó. Bước chạy đúng một lần.
    pub(crate) async fn approve_run(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Run, Error> {
        let run = self.awaiting(tenant, principal, id).await?;
        let decider = principal.user_id();
        self.requeue(
            tenant,
            &run,
            |update| update.col_expr(Column::DecidedBy, Expr::value(decider)),
            AWAITING_APPROVAL,
        )
        .await
    }

    /// Từ chối bước rủi ro đang chờ: lần chạy kết thúc `failed` (`automations.approval_rejected`).
    pub(crate) async fn reject_run(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Run, Error> {
        let mut run = self.awaiting(tenant, principal, id).await?;
        let mut steps: Vec<StepLog> = serde_json::from_str(&run.steps).unwrap_or_default();
        if let Some(step) = steps.iter_mut().find(|step| Some(step.index) == run.approval_step) {
            step.status = "rejected".to_owned();
            step.finished_at = Some(now());
        }
        run.steps = serde_json::to_string(&steps).map_err(Error::internal)?;
        run.decided_by = Some(principal.user_id());
        let error = Error::conflict(
            "automations.approval_rejected",
            "A person rejected a step of this run that needed approval.",
        );
        self.finish(tenant, &mut run, FAILED, Some(error)).await?;
        self.get_run(tenant, principal, id).await
    }

    /// Lần chạy `id` mà người gọi được chạy / chạy lại / duyệt.
    async fn runnable(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Run, Error> {
        principal.require(&AUTOMATIONS_RUN)?;
        let run = self.get_run(tenant, principal, id).await?;
        let target = autoxide_kernel::access::OrgTarget::new(
            autoxide_kernel::tenancy::OrgId::from_uuid(run.org_id),
            None,
        );
        principal.require_at(tenant.database().read(), &AUTOMATIONS_RUN, target).await?;
        Ok(run)
    }

    /// Lần chạy đang chờ duyệt mà người gọi quyết được.
    async fn awaiting(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Run, Error> {
        if principal.actor_kind() == ActorKind::Agent {
            return Err(human_required());
        }
        let run = self.runnable(tenant, principal, id).await?;
        if run.status != AWAITING_APPROVAL {
            return Err(run_state(&run.status, AWAITING_APPROVAL));
        }
        let mut errors = Vec::new();
        let actions =
            Action::parse_all(&parse_json(&run.definition)["actions"], "actions", &mut errors)
                .unwrap_or_default();
        let step = run
            .approval_step
            .and_then(|index| usize::try_from(index).ok())
            .and_then(|index| actions.get(index));
        if let Some(Action::Tool { tool, .. }) = step
            && let Some(tool) = self.directory.tool(tool)
        {
            // Người duyệt phải tự làm được bước đó, như gọi REST: kể cả xác nhận lại (step-up) cho
            // permission nhạy cảm (rà soát bảo mật 2026-11).
            tool.authorize(principal)?;
        }
        Ok(run)
    }

    /// Đưa lần chạy về `queued` (chỉ khi còn ở trạng thái `from`) và xếp job, cùng transaction.
    async fn requeue(
        &self,
        tenant: &TenantContext,
        run: &Run,
        extra: impl FnOnce(sea_orm::UpdateMany<Entity>) -> sea_orm::UpdateMany<Entity>,
        from: &str,
    ) -> Result<Run, Error> {
        let transaction = tenant.database().begin_write().await?;
        let update = Entity::update_many()
            .col_expr(Column::Status, Expr::value(QUEUED))
            .col_expr(Column::UpdatedAt, Expr::value(now()))
            .filter(Column::Id.eq(run.id))
            .filter(Column::Status.eq(from))
            .scoped(tenant);
        if extra(update).exec(&transaction).await?.rows_affected == 0 {
            transaction.rollback().await?;
            return Err(run_state("changed", from));
        }
        self.jobs
            .enqueue(&transaction, tenant, &RunJob { run_id: run.id }, EnqueueOptions::default())
            .await?;
        transaction.commit().await?;
        Entity::find_by_id(run.id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| run_not_found(run.id))
    }
}

/// Luật không đang bật: không chạy tay được.
fn rule_paused(status: &str) -> Error {
    Error::conflict(
        "automations.rule_paused",
        "The rule is `{status}`, not `active`: enable it before running it.",
    )
    .with_param("status", status.to_owned())
}

fn run_state(status: &str, expected: &str) -> Error {
    Error::conflict(
        "automations.run_state",
        "The run is `{status}`; this needs a run that is `{expected}`.",
    )
    .with_param("status", status.to_owned())
    .with_param("expected", expected.to_owned())
}
