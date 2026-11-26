//! Chạy luật — luôn trong job `automations.run`, ngoài transaction của event gây ra nó.
//!
//! 1. Kích hoạt (event, lịch, số liệu, gọi tay) gọi [`Automations::start_run`]: MỘT transaction ghi
//!    khoá chống chạy trùng `(khách, luật, khoá kích hoạt)`, dòng `automations_runs` và job. Nhận lại
//!    event thì khoá đã có → không chạy lần hai. Vượt số lần chạy mỗi giờ → luật tự tạm dừng.
//! 2. Job lấy lần chạy (chuyển `queued` / `waiting` → `running`; `running` quá hạn giữ thì lấy lại
//!    được khi máy chủ chết giữa chừng), dựng người chạy: quyền HIỆN TẠI của `run_as` (mất quyền
//!    của một tool → luật tự tắt), rồi chạy từng bước từ `cursor`.
//! 3. Mỗi bước xong là ghi lại ngay (`cursor`, nhật ký): chạy lại sau lỗi bắt đầu từ bước hỏng, bước
//!    đã xong không làm lại. Bước gọi tool mang `Idempotency-Key` `automation:<run_id>:<bước>` qua
//!    nơi lưu idempotency chung với REST.
//!
//! Người chạy mang đường vào `AccessPath::Automation(run_id)`: event do hành động sinh ra cho biết
//! lần chạy gây ra nó — luật không tự kích hoạt lại chính nó, chuỗi luật sâu tối đa
//! [`MAX_CHAIN_DEPTH`](crate::config::MAX_CHAIN_DEPTH).

pub(crate) mod schedule;
mod steps;
pub(crate) mod triggers;

use autoxide_kernel::access::{AccessPath, Principal};
use autoxide_kernel::error::{Error, ErrorKind};
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::{EnqueueOptions, Job, JobContext, JobHandler};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, EntityTrait, PaginatorTrait, QueryFilter, Set,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

use crate::definition::{Action, Condition as RuleCondition};
use crate::entity::{rule, run, run_key};
use crate::events::{self, Change};
use crate::service::rules::{ACTIVE, PAUSED, Rule, parse_json, snapshot};
use crate::service::{Automations, now};

/// Loại job chạy một lần chạy của luật.
pub(crate) const RUN_JOB: &str = "automations.run";
/// Lần chạy đang `running` mà không đổi trong chừng này phút thì coi như máy chủ đã chết giữa chừng.
const RUNNING_LEASE_MINUTES: i64 = 15;

/// Trạng thái của lần chạy.
pub(crate) const QUEUED: &str = "queued";
pub(crate) const RUNNING: &str = "running";
/// Số lần chạy đóng trong một câu UPDATE (`close_pending_runs`).
const CLOSE_BATCH: usize = 500;
pub(crate) const WAITING: &str = "waiting";
pub(crate) const AWAITING_APPROVAL: &str = "awaiting_approval";
pub(crate) const SUCCEEDED: &str = "succeeded";
pub(crate) const FAILED: &str = "failed";
pub(crate) const SKIPPED: &str = "skipped";

/// Payload của job: chỉ ID (dữ liệu nằm ở dòng của lần chạy).
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct RunJob {
    pub run_id: Uuid,
}

impl Job for RunJob {
    const KIND: &'static str = RUN_JOB;
}

/// One step in the log of a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub(crate) struct StepLog {
    /// Position of the action in the rule, from 0.
    pub index: i32,
    /// The tool name, or `wait`, `stop_if`, `notify`.
    pub action: String,
    /// `succeeded`, `failed`, `skipped`, `waiting`, `awaiting_approval`, `stopped` (a `stop_if`
    /// that held) or `rejected`.
    pub status: String,
    pub started_at: Timestamp,
    pub finished_at: Option<Timestamp>,
    /// The tool's result (cut down when large).
    #[schema(value_type = Option<Object>)]
    pub result: Option<Value>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    /// `automation:<run id>:<step>`: calling again never does the step twice.
    pub idempotency_key: Option<String>,
    /// Who approved the step, when it needed approval.
    pub approved_by: Option<Uuid>,
}

/// Một lần kích hoạt cần chạy.
#[derive(Debug, Clone)]
pub(crate) struct Start {
    /// `event:<id>`, `schedule:<mốc>`, `metric:<ngày>`, `manual:<id>`.
    pub trigger_key: String,
    pub event: Option<(Uuid, String)>,
    /// `{"event": …, "trigger": …}`.
    pub context: Value,
    pub depth: i32,
    pub triggered_by: Option<Uuid>,
    /// Ghi lần chạy `skipped` với mã lỗi này thay vì chạy (vd vòng lặp).
    pub skip: Option<(&'static str, &'static str)>,
}

/// Bộ chạy job `automations.run`.
pub(crate) struct RunHandler(pub Arc<Automations>);

#[async_trait::async_trait]
impl JobHandler for RunHandler {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let Some(tenant) = &job.tenant else { return Ok(()) };
        let payload: RunJob =
            serde_json::from_value(job.payload.clone()).map_err(Error::internal)?;
        self.0.execute(tenant, payload.run_id).await
    }
}

impl Automations {
    /// Ghi một lần chạy của `rule` và xếp job. `None`: kích hoạt này đã được xử lý (nhận lại event,
    /// lịch chạy hai lần) hay luật vừa tự tạm dừng vì chạy quá nhiều.
    pub(crate) async fn start_run(
        &self,
        tenant: &TenantContext,
        rule: &Rule,
        start: Start,
    ) -> Result<Option<run::Model>, Error> {
        let now = now();
        let id = Uuid::now_v7();
        let transaction = tenant.database().begin_write().await?;
        let key = run_key::ActiveModel {
            tenant_id: Set(tenant.id().as_uuid()),
            rule_id: Set(rule.id),
            trigger_key: Set(start.trigger_key.clone()),
            run_id: Set(id),
            created_at: Set(now),
        };
        let inserted = run_key::Entity::insert(key)
            .on_conflict(
                OnConflict::columns([
                    run_key::Column::TenantId,
                    run_key::Column::RuleId,
                    run_key::Column::TriggerKey,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec_without_returning(&transaction)
            .await?;
        if inserted == 0 {
            transaction.rollback().await?;
            return Ok(None);
        }
        // Giới hạn số lần chạy mỗi giờ của một luật (đếm trên index `(tenant, rule, created_at)`).
        // Khoá dòng luật trước khi đếm (D-181): nhiều kích hoạt cùng lúc từng cùng đếm thiếu trên
        // Postgres và vượt `max_runs_per_hour`.
        let locked = autoxide_kernel::rows::lock_one(
            &transaction,
            rule::Entity::find_by_id(rule.id).scoped(tenant),
        )
        .await?;
        // Bên gọi lọc luật qua kết nối đọc (có thể trễ): xem lại dưới khoá — luật vừa rời `active` không nhận lần chạy
        // mới, và lần chạy chụp đúng bản luật đang có (D-20261126-automation-owner-revoked).
        let Some(locked) = locked.filter(|row| row.status == crate::service::rules::ACTIVE) else {
            transaction.rollback().await?;
            return Ok(None);
        };
        let rule = &locked;
        let recent = run::Entity::find()
            .filter(run::Column::RuleId.eq(rule.id))
            .filter(run::Column::CreatedAt.gt(now - SignedDuration::from_hours(1)))
            .filter(run::Column::Status.ne(SKIPPED))
            .scoped(tenant)
            .count(&transaction)
            .await?;
        let over_limit = start.skip.is_none() && recent >= self.config.max_runs_per_hour;
        if over_limit {
            self.pause(&transaction, tenant, rule, "rate_limit").await?;
            transaction.commit().await?;
            tracing::warn!(rule_id = %rule.id, "automation rule paused: too many runs in an hour");
            return Ok(None);
        }
        let definition = json!({
            "conditions": parse_json(&rule.conditions),
            "actions": parse_json(&rule.actions),
        });
        let (status, error) = match start.skip {
            Some((code, message)) => (SKIPPED, Some((code, message))),
            None => (QUEUED, None),
        };
        let row = run::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant.id().as_uuid()),
            org_id: Set(rule.org_id),
            rule_id: Set(rule.id),
            rule_version: Set(rule.version),
            trigger_key: Set(start.trigger_key),
            event_id: Set(start.event.as_ref().map(|(id, _)| *id)),
            event_name: Set(start.event.map(|(_, name)| name)),
            depth: Set(start.depth),
            status: Set(status.to_owned()),
            cursor: Set(0),
            context: Set(start.context.to_string()),
            definition: Set(definition.to_string()),
            steps: Set("[]".to_owned()),
            error_code: Set(error.map(|(code, _)| code.to_owned())),
            error_message: Set(error.map(|(_, message)| message.to_owned())),
            approval_step: Set(None),
            decided_by: Set(None),
            triggered_by: Set(start.triggered_by),
            resume_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            finished_at: Set(error.map(|_| now)),
        }
        .insert(&transaction)
        .await?;
        if status == QUEUED {
            self.jobs
                .enqueue(&transaction, tenant, &RunJob { run_id: id }, EnqueueOptions::default())
                .await?;
        }
        transaction.commit().await?;
        Ok(Some(row))
    }

    /// Tạm dừng luật (trong `connection`, thường là transaction đang mở) và ghi event.
    /// API key `key` bị thu hồi: tạm dừng mọi luật đang bật mang quyền của nó (lý do
    /// `credential_revoked`). Nhận lại cùng event thì không còn luật nào đang bật: không làm gì.
    pub(crate) async fn credential_revoked(
        &self,
        tenant: &TenantContext,
        key: Uuid,
    ) -> Result<(), Error> {
        let rules = rule::Entity::find()
            .filter(rule::Column::RunAsApiKey.eq(key))
            .filter(rule::Column::Status.eq(ACTIVE))
            .scoped(tenant)
            .all(tenant.database().write())
            .await?;
        for rule in &rules {
            self.pause_now(tenant, rule, "credential_revoked").await?;
        }
        Ok(())
    }

    /// Mọi cửa vào tài khoản `user` bị cắt (event `auth.access_revoked`): tạm dừng mọi luật đang bật chạy bằng
    /// quyền của họ (lý do `owner_access_revoked`); bật lại cần một người có quyền (người bật thành `run_as`).
    /// Nhận lại cùng event thì không còn luật nào đang bật: không làm gì.
    pub(crate) async fn owner_revoked(
        &self,
        tenant: &TenantContext,
        user: Uuid,
    ) -> Result<(), Error> {
        let rules = rule::Entity::find()
            .filter(rule::Column::RunAs.eq(user))
            .filter(rule::Column::Status.eq(ACTIVE))
            .scoped(tenant)
            .all(tenant.database().write())
            .await?;
        for rule in &rules {
            self.pause_now(tenant, rule, "owner_access_revoked").await?;
        }
        if !rules.is_empty() {
            tracing::info!(user_id = %user, rules = rules.len(), "automation rules paused: owner access revoked");
        }
        Ok(())
    }

    /// [`pause`](Self::pause) trong transaction riêng: trạng thái luật, lần chạy dở và event ghi cùng lúc.
    pub(crate) async fn pause_now(
        &self,
        tenant: &TenantContext,
        rule: &Rule,
        reason: &str,
    ) -> Result<(), Error> {
        let transaction = tenant.database().begin_write().await?;
        self.pause(&transaction, tenant, rule, reason).await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Kết thúc mọi lần chạy đang chờ (`queued`, `waiting`, `awaiting_approval`) của luật `rule_id` vừa rời
    /// trạng thái `active` (sang `status`), với `automations.rule_not_active` và event `automations.run_failed`
    /// như lần chạy hỏng khác. Chạy trong `connection` của thay đổi trạng thái (thường là transaction đang mở).
    pub(crate) async fn close_pending_runs<C: sea_orm::ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        rule_id: Uuid,
        status: &str,
    ) -> Result<(), Error> {
        // Khoá các lần chạy đang chờ (Postgres `FOR UPDATE`, theo thứ tự id; SQLite: transaction ghi đã giữ cả
        // database) rồi đóng bằng MỘT câu UPDATE. Worker nhận lần chạy (`claim`) cùng lúc phải chờ khoá, rồi thấy trạng
        // thái đã đổi và bỏ qua; lần chạy worker nhận trước thì không còn ở trạng thái chờ nên không bị khoá, không bị
        // ghi đè — runner tự dừng nó trước bước kế tiếp. Mỗi lần chạy bị đóng đúng một event `RunFailed`.
        let pending = autoxide_kernel::rows::lock(
            connection,
            run::Entity::find()
                .filter(run::Column::RuleId.eq(rule_id))
                .filter(run::Column::Status.is_in([QUEUED, WAITING, AWAITING_APPROVAL]))
                .scoped(tenant),
            run::Column::Id,
        )
        .await?;
        if pending.is_empty() {
            return Ok(());
        }
        let error = rule_not_active(status);
        let at = now();
        // Chia lô: mỗi id là một tham số của câu lệnh; SQLite nhận tối đa 32766, Postgres 65535 tham số — luật có
        // hàng chục nghìn lần chạy chờ vẫn đóng được, trong cùng transaction.
        for batch in pending.chunks(CLOSE_BATCH) {
            run::Entity::update_many()
                .col_expr(run::Column::Status, Expr::value(FAILED))
                .col_expr(run::Column::ErrorCode, Expr::value(error.code()))
                .col_expr(run::Column::ErrorMessage, Expr::value(error.message()))
                .col_expr(run::Column::FinishedAt, Expr::value(Some(at)))
                .col_expr(run::Column::ResumeAt, Expr::value(Option::<Timestamp>::None))
                .col_expr(run::Column::UpdatedAt, Expr::value(at))
                .filter(run::Column::Id.is_in(batch.iter().map(|run| run.id)))
                .filter(run::Column::Status.is_in([QUEUED, WAITING, AWAITING_APPROVAL]))
                .scoped(tenant)
                .exec(connection)
                .await?;
        }
        for run in pending {
            let failure = json!({
                "run_id": run.id,
                "rule_id": run.rule_id,
                "error_code": error.code(),
                "error_message": error.message(),
            });
            let change = Change { before: None, after: Some(failure) };
            self.outbox
                .record(
                    connection,
                    tenant,
                    Actor::system(),
                    events::rule_subject(run.rule_id),
                    &events::RunFailed(change),
                )
                .await?;
        }
        Ok(())
    }

    pub(crate) async fn pause<C: sea_orm::ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        rule: &Rule,
        reason: &str,
    ) -> Result<(), Error> {
        let updated = rule::Entity::update_many()
            .col_expr(rule::Column::Status, Expr::value(PAUSED))
            .col_expr(rule::Column::StatusReason, Expr::value(reason))
            .col_expr(rule::Column::NextRunAt, Expr::value(Option::<Timestamp>::None))
            .col_expr(rule::Column::UpdatedAt, Expr::value(now()))
            .filter(rule::Column::Id.eq(rule.id))
            .filter(rule::Column::Status.eq(ACTIVE))
            .scoped(tenant)
            .exec(connection)
            .await?;
        if updated.rows_affected == 0 {
            return Ok(());
        }
        // Luật rời `active`: lần chạy đang chờ của nó kết thúc cùng lúc — không ai chạy tiếp chúng bằng quyền của
        // người bật lại sau này (D-20261126-automation-owner-revoked).
        self.close_pending_runs(connection, tenant, rule.id, PAUSED).await?;
        let mut after = snapshot(rule);
        after["status"] = Value::from(PAUSED);
        after["status_reason"] = Value::from(reason);
        let change = Change { before: Some(snapshot(rule)), after: Some(after) };
        self.outbox
            .record(
                connection,
                tenant,
                Actor::system(),
                events::rule_subject(rule.id),
                &events::RulePaused(change),
            )
            .await
    }

    /// Chạy (tiếp) lần chạy `id`. Lỗi trả về chỉ là lỗi hệ thống (job chạy lại sau); lỗi của một
    /// bước ghi vào lần chạy (`failed`).
    pub(crate) async fn execute(&self, tenant: &TenantContext, id: Uuid) -> Result<(), Error> {
        let Some(mut run) = self.claim(tenant, id).await? else { return Ok(()) };
        let rule = match self.current_rule(tenant, &run).await? {
            Ok(rule) => rule,
            Err(error) => return self.finish(tenant, &mut run, FAILED, Some(error)).await,
        };
        let definition = parse_json(&run.definition);
        let mut errors = Vec::new();
        let actions =
            Action::parse_all(&definition["actions"], "actions", &mut errors).unwrap_or_default();
        let principal = match self.runner(tenant, &rule, &actions).await? {
            Ok(principal) => principal.with_access(AccessPath::Automation(run.id)),
            Err(error) => return self.finish(tenant, &mut run, FAILED, Some(error)).await,
        };
        let mut steps: Vec<StepLog> = serde_json::from_str(&run.steps).unwrap_or_default();
        let mut context = parse_json(&run.context);
        while let Some(action) =
            usize::try_from(run.cursor).ok().and_then(|index| actions.get(index))
        {
            // Trước MỖI bước (không chỉ lúc bắt đầu): luật có thể vừa bị tạm dừng / tắt / sửa / bật lại giữa hai bước.
            if let Err(error) = self.current_rule(tenant, &run).await? {
                return self.finish(tenant, &mut run, FAILED, Some(error)).await;
            }
            context["steps"] = steps_context(&steps);
            match self.step(tenant, &principal, &mut run, action, &context).await? {
                steps::Outcome::Done(log) => {
                    put(&mut steps, log);
                    run.cursor += 1;
                    self.checkpoint(tenant, &mut run, &steps).await?;
                }
                steps::Outcome::Stop(log) => {
                    put(&mut steps, log);
                    self.checkpoint(tenant, &mut run, &steps).await?;
                    return self.finish(tenant, &mut run, SUCCEEDED, None).await;
                }
                steps::Outcome::Pause { log, status, resume_at } => {
                    put(&mut steps, log);
                    if status == WAITING {
                        run.cursor += 1;
                    }
                    run.status = status.to_owned();
                    run.resume_at = resume_at;
                    return self.suspend(tenant, &mut run, &steps).await;
                }
                steps::Outcome::Failed(log, error) => {
                    put(&mut steps, log);
                    self.checkpoint(tenant, &mut run, &steps).await?;
                    return self.finish(tenant, &mut run, FAILED, Some(error)).await;
                }
            }
        }
        self.finish(tenant, &mut run, SUCCEEDED, None).await
    }

    /// Lấy lần chạy để chạy: `queued` / `waiting`, hay `running` đã quá hạn giữ.
    async fn claim(&self, tenant: &TenantContext, id: Uuid) -> Result<Option<run::Model>, Error> {
        let now = now();
        let stale = now - SignedDuration::from_mins(RUNNING_LEASE_MINUTES);
        let claimed = run::Entity::update_many()
            .col_expr(run::Column::Status, Expr::value(RUNNING))
            .col_expr(run::Column::UpdatedAt, Expr::value(now))
            .filter(run::Column::Id.eq(id))
            .filter(
                Condition::any().add(run::Column::Status.is_in([QUEUED, WAITING])).add(
                    Condition::all()
                        .add(run::Column::Status.eq(RUNNING))
                        .add(run::Column::UpdatedAt.lt(stale)),
                ),
            )
            .scoped(tenant)
            .exec(tenant.database().write())
            .await?;
        if claimed.rows_affected == 0 {
            return Ok(None);
        }
        Ok(run::Entity::find_by_id(id).scoped(tenant).one(tenant.database().read()).await?)
    }

    /// Người chạy hành động: quyền HIỆN TẠI của `run_as`. Không còn tài khoản hay thiếu permission
    /// của một tool → luật tự tạm dừng (báo qua event `automations.rule_paused`), lần chạy hỏng.
    async fn runner(
        &self,
        tenant: &TenantContext,
        rule: &Rule,
        actions: &[Action],
    ) -> Result<Result<Principal, Error>, Error> {
        let principal = self.rule_principal(tenant, rule).await?;
        let Some(principal) = principal else {
            self.pause_now(tenant, rule, "owner_unavailable").await?;
            return Ok(Err(owner_unavailable()));
        };
        for action in actions {
            if let Action::Tool { tool, .. } = action {
                let allowed =
                    self.directory.tool(tool).is_some_and(|found| found.allows(&principal));
                if !allowed {
                    self.pause_now(tenant, rule, "permission_lost").await?;
                    return Ok(Err(Error::forbidden(
                        "automations.permission_missing",
                        "The author of the rule can no longer use tool `{tool}`; the rule is paused.",
                    )
                    .with_param("tool", tool.clone())));
                }
            }
        }
        Ok(Ok(principal))
    }

    /// Luật của lần chạy `run`, nếu lần chạy còn được chạy: luật còn `active` VÀ đúng phiên bản lúc tạo lần chạy.
    /// Sửa luật hay bật lại luật tăng `version` — bật lại đổi `run_as` sang người bật — nên lần chạy cũ (định nghĩa
    /// cũ, người chịu trách nhiệm cũ) không chạy bằng quyền của người mới (D-20261126-automation-owner-revoked).
    /// Đọc qua kết nối GHI: bản đọc có thể trễ sau lệnh tạm dừng vừa ghi.
    async fn current_rule(
        &self,
        tenant: &TenantContext,
        run: &run::Model,
    ) -> Result<Result<Rule, Error>, Error> {
        let rule = rule::Entity::find_by_id(run.rule_id)
            .scoped(tenant)
            .one(tenant.database().write())
            .await?;
        let Some(rule) = rule else { return Ok(Err(crate::service::rule_not_found(run.rule_id))) };
        if rule.status != crate::service::rules::ACTIVE {
            return Ok(Err(rule_not_active(&rule.status)));
        }
        if rule.version != run.rule_version {
            return Ok(Err(rule_changed()));
        }
        Ok(Ok(rule))
    }

    /// Ghi bước vừa xong (lần chạy vẫn `running`).
    async fn checkpoint(
        &self,
        tenant: &TenantContext,
        run: &mut run::Model,
        steps: &[StepLog],
    ) -> Result<(), Error> {
        run.steps = serde_json::to_string(steps).map_err(Error::internal)?;
        run.updated_at = now();
        self.save_run(tenant, run).await
    }

    /// Dừng chờ (`waiting` có hẹn giờ, `awaiting_approval`): ghi lại; `wait` thì xếp job hẹn giờ.
    async fn suspend(
        &self,
        tenant: &TenantContext,
        run: &mut run::Model,
        steps: &[StepLog],
    ) -> Result<(), Error> {
        run.steps = serde_json::to_string(steps).map_err(Error::internal)?;
        run.updated_at = now();
        let transaction = tenant.database().begin_write().await?;
        save(&transaction, tenant, run).await?;
        if let Some(resume_at) = run.resume_at.filter(|_| run.status == WAITING) {
            let options = EnqueueOptions::default().run_at(resume_at);
            self.jobs.enqueue(&transaction, tenant, &RunJob { run_id: run.id }, options).await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    /// Kết thúc lần chạy; hỏng thì ghi event `automations.run_failed` cùng transaction.
    pub(crate) async fn finish(
        &self,
        tenant: &TenantContext,
        run: &mut run::Model,
        status: &str,
        error: Option<Error>,
    ) -> Result<(), Error> {
        let error = match error {
            // Lỗi hệ thống: để job chạy lại (bước đã xong không làm lại).
            Some(error) if error.kind() == ErrorKind::Internal => return Err(error),
            other => other,
        };
        let transaction = tenant.database().begin_write().await?;
        self.finish_in(&transaction, tenant, run, status, error).await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Phần ghi của [`finish`](Self::finish) trong `connection` (transaction của bên gọi).
    async fn finish_in<C: sea_orm::ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        run: &mut run::Model,
        status: &str,
        error: Option<Error>,
    ) -> Result<(), Error> {
        let now = now();
        run.status = status.to_owned();
        run.error_code = error.as_ref().map(|error| error.code().to_owned());
        run.error_message = error.as_ref().map(Error::message);
        run.finished_at = Some(now);
        run.resume_at = None;
        run.updated_at = now;
        save(connection, tenant, run).await?;
        if status == FAILED {
            let failure = json!({
                "run_id": run.id,
                "rule_id": run.rule_id,
                "error_code": run.error_code,
                "error_message": run.error_message,
            });
            let change = Change { before: None, after: Some(failure) };
            self.outbox
                .record(
                    connection,
                    tenant,
                    Actor::system(),
                    events::rule_subject(run.rule_id),
                    &events::RunFailed(change),
                )
                .await?;
        }
        Ok(())
    }

    async fn save_run(&self, tenant: &TenantContext, run: &run::Model) -> Result<(), Error> {
        save(tenant.database().write(), tenant, run).await
    }

    /// Điều kiện của luật có đúng với dữ liệu kích hoạt `context` không.
    pub(crate) fn matches(conditions: Option<&RuleCondition>, context: &Value) -> bool {
        conditions.is_none_or(|condition| condition.holds(context))
    }
}

async fn save<C: sea_orm::ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    run: &run::Model,
) -> Result<(), Error> {
    let row: run::ActiveModel = run.clone().into();
    let row = row.reset_all();
    run::Entity::update(row).validate()?.scoped(tenant).exec(connection).await?;
    Ok(())
}

/// Đặt nhật ký của bước `log.index` (thay bản cũ của cùng bước, vd lúc chờ duyệt).
fn put(steps: &mut Vec<StepLog>, log: StepLog) {
    match steps.iter_mut().find(|known| known.index == log.index) {
        Some(known) => *known = log,
        None => steps.push(log),
    }
}

/// `steps` cho mẫu: theo thứ tự hành động, `{"status", "result"}`.
fn steps_context(steps: &[StepLog]) -> Value {
    Value::Array(
        steps.iter().map(|step| json!({"status": step.status, "result": step.result})).collect(),
    )
}

fn rule_not_active(status: &str) -> Error {
    Error::conflict(
        "automations.rule_not_active",
        "The rule became `{status}` before this run finished, so the run stopped. Check the rule, \
         enable it again and run it if needed.",
    )
    .with_param("status", status.to_owned())
}

fn rule_changed() -> Error {
    Error::conflict(
        "automations.rule_changed",
        "The rule was changed or enabled again after this run started, so the run stopped: it would \
         otherwise finish the old actions under the new person responsible. Run the rule again if \
         needed.",
    )
}

fn owner_unavailable() -> Error {
    Error::forbidden(
        "automations.owner_unavailable",
        "The person this rule runs as no longer has an active account here; the rule is paused. A \
         person with `automations.manage` can enable it again to run it as themselves.",
    )
}
