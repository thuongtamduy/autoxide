//! Chạy MỘT bước của lần chạy.

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, ErrorKind};
use autoxide_kernel::idempotency::{Begin, StoredResponse};
use autoxide_kernel::tenancy::TenantContext;
use chrono::{DateTime, Duration, Utc};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use super::{AWAITING_APPROVAL, StepLog, WAITING};
use crate::definition::{Action, fill};
use crate::entity::run;
use crate::service::{Automations, now};

/// Kết quả nhật ký giữ tối đa chừng này ký tự JSON; dài hơn thì cắt (vẫn đọc được phần đầu).
const RESULT_MAX_CHARS: usize = 4000;
/// Một lần gọi tool giữ quyền chạy key idempotency tối đa chừng này phút.
const STEP_LOCK_MINUTES: i64 = 10;

/// Kết quả của một bước.
pub(super) enum Outcome {
    /// Xong, sang bước sau.
    Done(StepLog),
    /// `stop_if` đúng: lần chạy kết thúc thành công.
    Stop(StepLog),
    /// Dừng chờ: `wait` (hẹn giờ) hay bước rủi ro chờ người duyệt.
    Pause { log: StepLog, status: &'static str, resume_at: Option<DateTime<Utc>> },
    /// Bước hỏng: lần chạy kết thúc `failed` với lỗi này.
    Failed(StepLog, Error),
}

impl Automations {
    pub(super) async fn step(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        run: &mut run::Model,
        action: &Action,
        context: &Value,
    ) -> Result<Outcome, Error> {
        let started_at = now();
        let mut log = StepLog {
            index: run.cursor,
            action: action.kind().to_owned(),
            status: "succeeded".to_owned(),
            started_at,
            finished_at: Some(started_at),
            result: None,
            error_code: None,
            error_message: None,
            idempotency_key: None,
            approved_by: None,
        };
        match action {
            Action::Wait { minutes } => {
                log.status = "waiting".to_owned();
                let minutes = i64::try_from(*minutes).unwrap_or(i64::MAX / 120);
                let resume_at = started_at + Duration::minutes(minutes);
                log.result = Some(json!({"resume_at": resume_at}));
                Ok(Outcome::Pause { log, status: WAITING, resume_at: Some(resume_at) })
            }
            Action::StopIf(condition) => {
                if condition.holds(context) {
                    log.status = "stopped".to_owned();
                    Ok(Outcome::Stop(log))
                } else {
                    Ok(Outcome::Done(log))
                }
            }
            Action::Notify { title, body } => {
                self.notify(tenant, run, log, title, body, context).await
            }
            Action::Tool { tool, arguments } => {
                self.call_tool(tenant, principal, run, log, (tool, arguments), context).await
            }
        }
    }

    async fn notify(
        &self,
        tenant: &TenantContext,
        run: &run::Model,
        mut log: StepLog,
        title: &str,
        body: &str,
        context: &Value,
    ) -> Result<Outcome, Error> {
        let Some(notifier) = &self.notifier else {
            log.status = "skipped".to_owned();
            log.result = Some(json!({"reason": "notifications are not installed"}));
            return Ok(Outcome::Done(log));
        };
        let title = match fill::fill(&self.templates, &Value::from(title), context) {
            Ok(value) => text(value),
            Err(error) => return Ok(failed(log, error)),
        };
        let body = match fill::fill(&self.templates, &Value::from(body), context) {
            Ok(value) => text(value),
            Err(error) => return Ok(failed(log, error)),
        };
        let Some(owner) = self.owner_of(tenant, run).await? else {
            log.status = "skipped".to_owned();
            return Ok(Outcome::Done(log));
        };
        let notification = autoxide_notifications::api::Notification::new(
            crate::notices::ALERT.key(),
            autoxide_notifications::api::Recipient::User(owner),
            json!({"title": title, "body": body}),
            format!("automation:{}:{}", run.id, log.index),
        );
        match notifier.notify(tenant.id(), notification).await {
            Ok(_) => {
                log.result = Some(json!({"title": title}));
                Ok(Outcome::Done(log))
            }
            Err(error) if error.kind() == ErrorKind::Internal => Err(error),
            Err(error) => Ok(failed(log, error)),
        }
    }

    /// Người nhận thông báo của luật: `run_as` của nó.
    async fn owner_of(
        &self,
        tenant: &TenantContext,
        run: &run::Model,
    ) -> Result<Option<uuid::Uuid>, Error> {
        use autoxide_kernel::tenancy::ScopeExt;
        use sea_orm::EntityTrait;
        let rule = crate::entity::rule::Entity::find_by_id(run.rule_id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?;
        Ok(rule.map(|rule| rule.run_as))
    }

    /// Bước gọi tool `name` với `arguments` (còn mẫu, điền bằng `context`).
    async fn call_tool(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        run: &mut run::Model,
        mut log: StepLog,
        (name, arguments): (&str, &Map<String, Value>),
        context: &Value,
    ) -> Result<Outcome, Error> {
        let key = format!("automation:{}:{}", run.id, log.index);
        log.idempotency_key = Some(key.clone());
        let Some(tool) = self.directory.tool(name) else {
            return Ok(failed(log, tool_gone(name)));
        };
        // Lệnh rủi ro (tool xoá / ghi đè, hay cần người duyệt khi agent gọi): chờ người duyệt; mỗi
        // lần duyệt cho chạy đúng một bước.
        let risky = tool.is_destructive() || tool.needs_approval_for_agent();
        let approved = run.approval_step == Some(log.index) && run.decided_by.is_some();
        if risky && !approved {
            run.approval_step = Some(log.index);
            run.decided_by = None;
            log.status = AWAITING_APPROVAL.to_owned();
            log.finished_at = None;
            return Ok(Outcome::Pause { log, status: AWAITING_APPROVAL, resume_at: None });
        }
        if approved {
            log.approved_by = run.decided_by;
            run.approval_step = None;
        }
        let arguments =
            match fill::fill(&self.templates, &Value::Object(arguments.clone()), context) {
                Ok(arguments) => arguments,
                Err(error) => return Ok(failed(log, error)),
            };
        let outcome = self.call_once(tenant, principal, tool, &key, arguments).await;
        log.finished_at = Some(now());
        match outcome {
            Ok(result) => {
                log.result = Some(shorten(result));
                Ok(Outcome::Done(log))
            }
            Err(error) if error.kind() == ErrorKind::Internal => Err(error),
            Err(error) => Ok(failed(log, error)),
        }
    }

    /// Gọi tool một lần theo key idempotency `key` (nơi lưu chung với REST): đã có kết quả thì trả
    /// lại, không gọi lần hai. Không cài nơi lưu thì gọi thẳng — bước đã ghi xong vẫn không chạy lại.
    async fn call_once(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        tool: &autoxide_kernel::mcp::Tool,
        key: &str,
        arguments: Value,
    ) -> Result<Value, Error> {
        let Some(store) = &self.idempotency else {
            return self.directory.call(tool, tenant.clone(), principal.clone(), arguments).await;
        };
        let scope = format!("automation:{}", principal.user_id());
        let fingerprint = fingerprint(tool.name(), &arguments);
        let locked_until = now() + Duration::minutes(STEP_LOCK_MINUTES);
        match store.begin(tenant, &scope, key, &fingerprint, locked_until).await? {
            Begin::Replay(stored) => serde_json::from_slice(&stored.body).map_err(Error::internal),
            Begin::Started { id } => {
                let result =
                    self.directory.call(tool, tenant.clone(), principal.clone(), arguments).await;
                match &result {
                    Ok(value) => {
                        let body = serde_json::to_vec(value).map_err(Error::internal)?;
                        let stored = StoredResponse::new(200, Vec::new(), body);
                        store.complete(tenant, id, stored).await?;
                    }
                    // Bước hỏng không đổi gì: bỏ ghi nhận để chạy lại được.
                    Err(_) => store.abandon(tenant, id).await?,
                }
                result
            }
            // Lần gọi trước còn đang chạy (máy chủ khác, hay vừa chết): job chạy lại sau.
            Begin::InProgress => Err(Error::internal("the step is still running elsewhere")),
            Begin::Mismatch => Err(Error::conflict(
                "automations.step_changed",
                "This step already ran with other arguments; the run cannot continue.",
            )),
            _ => Err(Error::internal("unknown idempotency outcome")),
        }
    }
}

fn failed(mut log: StepLog, error: Error) -> Outcome {
    log.status = "failed".to_owned();
    log.finished_at = Some(now());
    log.error_code = Some(error.code().to_owned());
    log.error_message = Some(error.message());
    Outcome::Failed(log, error)
}

fn tool_gone(name: &str) -> Error {
    Error::conflict(
        "automations.tool_unknown",
        "Tool `{tool}` is no longer available (its module was removed or is not in the plan).",
    )
    .with_param("tool", name.to_owned())
}

fn text(value: Value) -> String {
    match value {
        Value::String(text) => text,
        other => other.to_string(),
    }
}

/// Kết quả cho nhật ký: cắt bớt khi quá dài.
fn shorten(result: Value) -> Value {
    let text = result.to_string();
    if text.chars().count() <= RESULT_MAX_CHARS {
        return result;
    }
    let preview: String = text.chars().take(RESULT_MAX_CHARS).collect();
    json!({"truncated": true, "preview": preview})
}

fn fingerprint(tool: &str, arguments: &Value) -> String {
    let mut digest = Sha256::new();
    digest.update(tool.as_bytes());
    digest.update([0]);
    digest.update(arguments.to_string().as_bytes());
    digest.finalize().iter().map(|byte| format!("{byte:02x}")).collect()
}
