//! Chống lạm dụng: nhân viên chặn người mua viết tới nút của cuộc trò chuyện (gỡ chặn được), người
//! mua hay nhân viên báo vi phạm một cuộc / một tin, người có `conversations.manage` xem và xử lý báo
//! cáo. Không đọc tin để làm việc gì khác.

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use sea_orm::sea_query::{Condition, Expr, OnConflict};
use sea_orm::{ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter};
use serde_json::json;
use uuid::Uuid;

use crate::entity::{block, message, report};
use crate::events::{self, Change};
use crate::permissions::MANAGE;
use crate::service::{ConversationsService, Viewer, finish, now, target};
use crate::threads::actor_of;

/// Độ dài tối đa của lý do.
const REASON_MAX: usize = 1000;
const BLOCK_REASON_MAX: usize = 500;

pub(crate) type Report = report::Model;

impl ConversationsService {
    /// Chặn (`blocked = true`) hay gỡ chặn người mua của cuộc `id` viết tới nút của cuộc đó
    /// (`conversations.manage`). Gọi lại không đổi gì.
    pub(crate) async fn set_block(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        blocked: bool,
        reason: &str,
    ) -> Result<bool, Error> {
        let thread = self.get(tenant, Viewer::Staff(principal), id).await?;
        principal.require_at(tenant.database().read(), &MANAGE, target(&thread)).await?;
        let reason = reason.trim();
        if reason.chars().count() > BLOCK_REASON_MAX {
            return Err(Error::validation(vec![
                FieldError::new("reason", "length", "must be at most {max} characters")
                    .with_param("max", BLOCK_REASON_MAX),
            ]));
        }
        let Some(customer) = thread.customer_user_id else {
            return Ok(false);
        };
        let transaction = tenant.database().begin_write().await?;
        let changed = if blocked {
            block::Entity::insert(
                block::Model {
                    id: Uuid::now_v7(),
                    tenant_id: tenant.id().as_uuid(),
                    org_id: thread.org_id,
                    user_id: customer,
                    reason: reason.to_owned(),
                    blocked_by: Some(principal.user_id()),
                    created_at: now(),
                }
                .into_active_model(),
            )
            .on_conflict(
                OnConflict::columns([
                    block::Column::TenantId,
                    block::Column::OrgId,
                    block::Column::UserId,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec_without_returning(&transaction)
            .await?
        } else {
            block::Entity::delete_many()
                .filter(block::Column::OrgId.eq(thread.org_id))
                .filter(block::Column::UserId.eq(customer))
                .scoped(tenant)
                .exec(&transaction)
                .await?
                .rows_affected
        };
        if changed > 0 {
            let event = events::CustomerBlocked(Change {
                before: None,
                after: Some(json!({"org_id": thread.org_id, "user_id": customer, "blocked": blocked})),
                audience: Vec::new(),
            });
            let subject = events::thread_subject(id);
            self.parts.outbox.record(&transaction, tenant, Actor::from(principal), subject, &event).await?;
        }
        transaction.commit().await?;
        Ok(blocked)
    }

    /// Báo vi phạm cuộc `id` (và tin `message_id` nếu có) — người mua của cuộc hay nhân viên đọc được
    /// nó.
    pub(crate) async fn report(
        &self,
        tenant: &TenantContext,
        viewer: Viewer<'_>,
        id: Uuid,
        message_id: Option<Uuid>,
        reason: &str,
    ) -> Result<Report, Error> {
        let thread = self.get(tenant, viewer, id).await?;
        let mut problems = Vec::new();
        let reason = reason.trim();
        if !(1..=REASON_MAX).contains(&reason.chars().count()) {
            problems.push(
                FieldError::new("reason", "length", "must be 1 to {max} characters")
                    .with_param("max", REASON_MAX),
            );
        }
        if let Some(message_id) = message_id {
            let known = message::Entity::find_by_id(message_id)
                .filter(message::Column::ThreadId.eq(id))
                .scoped(tenant)
                .one(tenant.database().read())
                .await?;
            if known.is_none() {
                problems.push(FieldError::new(
                    "message_id",
                    "unknown",
                    "is not a message of this conversation",
                ));
            }
        }
        finish(problems)?;
        let row = report::Model {
            id: Uuid::now_v7(),
            tenant_id: tenant.id().as_uuid(),
            org_id: thread.org_id,
            thread_id: id,
            message_id,
            reporter_kind: if viewer.is_staff() { "agent" } else { "customer" }.to_owned(),
            reporter_user_id: Some(viewer.user_id()),
            reason: reason.to_owned(),
            status: "open".to_owned(),
            resolved_by: None,
            resolved_at: None,
            created_at: now(),
        };
        let transaction = tenant.database().begin_write().await?;
        report::Entity::insert(row.clone().into_active_model())
            .exec_without_returning(&transaction)
            .await?;
        let event = events::ReportCreated(Change {
            before: None,
            after: Some(report_snapshot(&row)),
            audience: Vec::new(),
        });
        let subject = events::thread_subject(id);
        self.parts.outbox.record(&transaction, tenant, actor_of(viewer), subject, &event).await?;
        transaction.commit().await?;
        Ok(row)
    }

    /// Một trang báo vi phạm trong phạm vi `conversations.manage`, mới trước (mặc định).
    pub(crate) async fn reports(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        status: Option<&str>,
        page: &PageRequest,
    ) -> Result<Page<Report>, Error> {
        let scope = principal.scope_condition(&MANAGE, report::Column::OrgId, None::<report::Column>)?;
        let mut condition = Condition::all().add(scope);
        if let Some(status) = status {
            condition = condition.add(report::Column::Status.eq(status));
        }
        let select = report::Entity::find().filter(condition).scoped(tenant);
        fetch_page(select, report::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    /// Đánh dấu báo cáo `id` đã xử lý (`conversations.manage` ở nút của nó).
    pub(crate) async fn resolve_report(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Report, Error> {
        let missing = || {
            Error::not_found(
                "conversations.report_not_found",
                "There is no report `{id}` you can see.",
            )
            .with_param("id", id)
        };
        let read = tenant.database().read();
        let before =
            report::Entity::find_by_id(id).scoped(tenant).one(read).await?.ok_or_else(missing)?;
        let at = autoxide_kernel::access::OrgTarget::new(OrgId::from_uuid(before.org_id), None);
        principal.require_at(read, &MANAGE, at).await.map_err(|error| {
            if error.code() == "org_out_of_scope" { missing() } else { error }
        })?;
        if before.status == "resolved" {
            return Ok(before);
        }
        let transaction = tenant.database().begin_write().await?;
        let moment = now();
        report::Entity::update_many()
            .col_expr(report::Column::Status, Expr::value("resolved"))
            .col_expr(report::Column::ResolvedBy, Expr::value(Some(principal.user_id())))
            .col_expr(report::Column::ResolvedAt, Expr::value(Some(moment)))
            .filter(report::Column::Id.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        let mut after = before.clone();
        after.status = "resolved".to_owned();
        after.resolved_by = Some(principal.user_id());
        after.resolved_at = Some(moment);
        let event = events::ReportResolved(Change {
            before: Some(report_snapshot(&before)),
            after: Some(report_snapshot(&after)),
            audience: Vec::new(),
        });
        let subject = events::thread_subject(before.thread_id);
        self.parts.outbox.record(&transaction, tenant, Actor::from(principal), subject, &event).await?;
        transaction.commit().await?;
        Ok(after)
    }
}

fn report_snapshot(row: &Report) -> serde_json::Value {
    json!({
        "id": row.id,
        "org_id": row.org_id,
        "thread_id": row.thread_id,
        "message_id": row.message_id,
        "reporter_kind": row.reporter_kind,
        "status": row.status,
    })
}
