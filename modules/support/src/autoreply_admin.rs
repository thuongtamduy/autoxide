//! Quản lý trả lời tự động (D-230): duyệt câu trả lời mẫu, xem thử, nhật ký, phản hồi "hữu ích /
//! không đúng" của người mua, nút "gặp nhân viên".

use autoxide_kernel::time::{Date, Timestamp};
use jiff::SignedDuration;
use std::collections::BTreeMap;

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use sea_orm::{ColumnTrait, EntityTrait, IntoActiveModel, PaginatorTrait, QueryFilter, Set};
use serde_json::json;
use uuid::Uuid;

use crate::api::{InboundQuestion, ReplyDecision, Requester};
use crate::entity::{auto_reply, saved_reply, ticket};
use crate::events::{self, Change};
use crate::extras::{SavedReply, saved_reply_not_found};
use crate::permissions::AUTO_REPLY_MANAGE;
use crate::service::{SupportService, Ticket, Viewer, finish, not_found, now};
use crate::status::Status;

pub(crate) type AutoReply = auto_reply::Model;

impl SupportService {
    /// Bật / tắt trả lời tự động cho câu trả lời mẫu `id` (`support.auto_reply.manage` ở nút của
    /// nó). Bật = người gọi duyệt nguyên văn câu trả lời, kèm cách hỏi mẫu và ngày phải rà lại.
    pub(crate) async fn approve_reply(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        enabled: bool,
        questions: BTreeMap<String, Vec<String>>,
        review_due_on: Option<Date>,
    ) -> Result<SavedReply, Error> {
        let today = Timestamp::now().date_utc();
        if enabled {
            let mut problems = Vec::new();
            let count: usize = questions.values().map(Vec::len).sum();
            if count == 0
                || count > 50
                || questions
                    .values()
                    .flatten()
                    .any(|q| q.trim().is_empty() || q.chars().count() > 300)
            {
                problems.push(FieldError::new(
                    "questions",
                    "invalid",
                    "give 1 to 50 example questions per language, each 1 to 300 characters",
                ));
            }
            finish(problems)?;
            if review_due_on.is_none_or(|day| {
                day < today || today.checked_add_days(366).is_some_and(|limit| day > limit)
            }) {
                return Err(Error::new(
                    ErrorKind::Validation,
                    "support.macro_not_approved",
                    "Give `review_due_on`: a day from today to one year ahead when someone checks this answer again.",
                )
                .with_field(FieldError::new("review_due_on", "required", "a day from today to one year ahead")));
            }
        }
        let transaction = tenant.database().begin_write().await?;
        let current = saved_reply::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| saved_reply_not_found(id))?;
        let target = OrgTarget::new(OrgId::from_uuid(current.org_id), None);
        principal.require_write_at(&transaction, &AUTO_REPLY_MANAGE, target).await?;
        let mut row = current.into_active_model();
        row.auto_reply = Set(enabled);
        if enabled {
            row.questions = Set(serde_json::to_string(&questions).map_err(Error::internal)?);
            row.approved_by = Set(Some(principal.user_id()));
            row.approved_at = Set(Some(now()));
            row.review_due_on = Set(review_due_on.map(|day| day.to_string()));
        }
        row.updated_at = Set(now());
        let after =
            saved_reply::Entity::update(row).validate()?.scoped(tenant).exec(&transaction).await?;
        self.settings_event(
            &transaction,
            tenant,
            principal,
            json!({"saved_reply": id, "auto_reply": enabled}),
        )
        .await?;
        transaction.commit().await?;
        Ok(after)
    }

    /// Xem thử: quyết định trả lời tự động cho câu `text` ở nút `org` mà không gửi gì.
    pub(crate) async fn preview(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org: Uuid,
        text: &str,
        locale: &str,
    ) -> Result<ReplyDecision, Error> {
        principal
            .require_at(
                tenant.database().read(),
                &AUTO_REPLY_MANAGE,
                OrgTarget::new(OrgId::from_uuid(org), None),
            )
            .await?;
        if self.parts.ai.is_none() {
            return Err(auto_reply_unavailable());
        }
        let question = InboundQuestion {
            org_id: org,
            thread: "support.preview".to_owned(),
            requester: Requester::User(principal.user_id()),
            text: text.to_owned(),
            locale: locale.to_owned(),
            auto_replies_in_row: 0,
        };
        self.decide(tenant, &question, None).await
    }

    /// Nhật ký trả lời tự động của nút `org`, mới trước.
    pub(crate) async fn auto_replies(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        page: &PageRequest,
    ) -> Result<Page<AutoReply>, Error> {
        let scope =
            principal.scope_condition(&AUTO_REPLY_MANAGE, auto_reply::Column::OrgId, None)?;
        let select = auto_reply::Entity::find().filter(scope).scoped(tenant);
        fetch_page(select, auto_reply::Column::Id, |row| row.id, page, tenant.database().read())
            .await
    }

    /// Người mua báo câu trả lời tự động `reply_id` của phiếu là hữu ích hay không đúng. Câu trả lời
    /// mẫu bị báo "không đúng" từ `AUTOXIDE_SUPPORT_AUTO_REPLY_FLAG_LIMIT` lần trong 7 ngày thì thôi dùng.
    pub(crate) async fn feedback(
        &self,
        tenant: &TenantContext,
        viewer: Viewer<'_>,
        ticket_id: Uuid,
        reply_id: Uuid,
        helpful: bool,
    ) -> Result<AutoReply, Error> {
        self.get(tenant, viewer, ticket_id).await?;
        let transaction = tenant.database().begin_write().await?;
        let current = auto_reply::Entity::find_by_id(reply_id)
            .filter(auto_reply::Column::Thread.eq(format!("support.ticket:{ticket_id}")))
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| reply_not_found(reply_id))?;
        let mut row = current.clone().into_active_model();
        row.feedback = Set(Some(if helpful { "helpful" } else { "wrong" }.to_owned()));
        let after =
            auto_reply::Entity::update(row).validate()?.scoped(tenant).exec(&transaction).await?;
        if !helpful && let Some(macro_id) = current.macro_id {
            let since = Timestamp::now() - SignedDuration::from_hours(24 * 7);
            let wrong = auto_reply::Entity::find()
                .filter(auto_reply::Column::MacroId.eq(macro_id))
                .filter(auto_reply::Column::Feedback.eq("wrong"))
                .filter(auto_reply::Column::CreatedAt.gte(since))
                .scoped(tenant)
                .count(&transaction)
                .await?;
            if wrong >= u64::from(self.config.auto_reply_flag_limit) {
                let disabled = saved_reply::Entity::update_many()
                    .col_expr(
                        saved_reply::Column::AutoReply,
                        sea_orm::sea_query::Expr::value(false),
                    )
                    .filter(saved_reply::Column::Id.eq(macro_id))
                    .filter(saved_reply::Column::AutoReply.eq(true))
                    .scoped(tenant)
                    .exec(&transaction)
                    .await?;
                if disabled.rows_affected > 0 {
                    let event = events::AutoReplyDisabled(Change {
                        before: None,
                        after: Some(json!({"macro_id": macro_id, "reason": "reported_wrong"})),
                    });
                    self.parts
                        .outbox
                        .record(&transaction, tenant, Actor::system(), None, &event)
                        .await?;
                }
            }
        }
        transaction.commit().await?;
        Ok(after)
    }

    /// Người mua bấm "gặp nhân viên": thôi trả lời tự động trong phiếu, phiếu chờ nhân viên.
    pub(crate) async fn handoff(
        &self,
        tenant: &TenantContext,
        viewer: Viewer<'_>,
        ticket_id: Uuid,
    ) -> Result<Ticket, Error> {
        self.get(tenant, viewer, ticket_id).await?;
        let transaction = tenant.database().begin_write().await?;
        let before = ticket::Entity::find_by_id(ticket_id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| not_found(ticket_id))?;
        if before.status == Status::Closed.as_str() {
            return Err(crate::status::ticket_closed());
        }
        let mut row = before.clone().into_active_model();
        row.auto_reply_paused = Set(true);
        if before.status != Status::Open.as_str() {
            let policy = self.policy(tenant, before.org_id).await?.policy;
            crate::sla::transition(&mut row, &before, Status::PendingAgent, None, &policy, now());
        }
        row.updated_at = Set(now());
        let after =
            ticket::Entity::update(row).validate()?.scoped(tenant).exec(&transaction).await?;
        self.system_line(&transaction, tenant, ticket_id, "handoff", now()).await?;
        self.record_changes(&transaction, tenant, Actor::public(), &before, &after, false).await?;
        transaction.commit().await?;
        Ok(after)
    }
}

pub(crate) fn auto_reply_unavailable() -> Error {
    Error::unavailable(
        "support.auto_reply_unavailable",
        "Automatic answers are not available: module `ai` is not installed or not set up.",
    )
}

fn reply_not_found(id: Uuid) -> Error {
    Error::not_found("support.auto_reply_not_found", "This ticket has no automatic answer `{id}`.")
        .with_param("id", id)
}
