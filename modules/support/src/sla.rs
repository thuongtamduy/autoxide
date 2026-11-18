//! Hạn trả lời (SLA) của phiếu (bước 53 mục 3): tính theo giờ làm việc và ngày nghỉ của nút, dừng
//! khi chờ người mua; job đánh dấu quá hạn và tự đóng phiếu đã xử lý; nguồn việc cho hộp việc.
//!
//! - `first_response_due_at`: lúc tạo + phút phản hồi đầu theo ưu tiên (phút làm việc).
//! - `resolve_due_at`: lúc tạo + phút giải quyết theo ưu tiên + phút làm việc đã chờ người mua
//!   (`sla_paused_minutes`). Đổi ưu tiên thì tính lại từ lúc tạo.
//! - Chờ người mua (`pending_customer`): `sla_paused_at` = lúc bắt đầu chờ; người mua viết lại thì
//!   cộng phút làm việc đã chờ vào `sla_paused_minutes` và dời hạn giải quyết.

use std::collections::HashMap;
use std::sync::Arc;

use autoxide_kernel::access::{OrgTarget, Permission, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::{JobContext, JobHandler};
use autoxide_kernel::pagination::{Order, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{DatabaseRegistry, ScopeExt, TenantContext, TenantId};
use autoxide_kernel::work::{Severity, WorkItem, WorkSource};
use chrono::{DateTime, Utc};
use sea_orm::sea_query::Condition;
use sea_orm::{
    ColumnTrait, EntityTrait, IntoActiveModel, PaginatorTrait, QueryFilter, QuerySelect, Set,
};
use uuid::Uuid;

use crate::entity::ticket;
use crate::events::{self, Change};
use crate::permissions::TICKETS_READ;
use crate::policy::Policy;
use crate::service::{SupportService, Ticket, now};
use crate::status::{Priority, Resolution, Status};
use crate::tickets::{apply_status, snapshot};

pub(crate) const SLA_CHECK: &str = "support.sla_check";
pub(crate) const AUTO_CLOSE: &str = "support.auto_close";
/// Số phiếu xét mỗi vòng của job.
const BATCH: u64 = 500;
/// Trạng thái còn mở (đồng hồ có thể chạy).
const ACTIVE: [&str; 3] = ["open", "pending_agent", "pending_customer"];

/// Hạn của một phiếu theo chính sách: (phản hồi đầu, giải quyết). `None` khi lịch không có giờ làm.
pub(crate) fn due_dates(
    policy: &Policy,
    created_at: DateTime<Utc>,
    priority: Priority,
    paused_minutes: i64,
) -> (Option<DateTime<Utc>>, Option<DateTime<Utc>>) {
    let Some(calendar) = policy.calendar() else {
        return (None, None);
    };
    let first =
        policy.first_response(priority).and_then(|minutes| calendar.add(created_at, minutes));
    let resolve = policy
        .resolve(priority)
        .and_then(|minutes| calendar.add(created_at, minutes + paused_minutes));
    (first, resolve)
}

/// Đặt trạng thái `to` và giữ đồng hồ đúng: chờ người mua thì dừng; hết chờ thì cộng thời gian
/// đã chờ và dời hạn giải quyết.
pub(crate) fn transition(
    row: &mut ticket::ActiveModel,
    before: &Ticket,
    to: Status,
    resolution: Option<Resolution>,
    policy: &Policy,
    at: DateTime<Utc>,
) {
    apply_status(row, before, to, resolution, at);
    let resumes = before.sla_paused_at.is_some() && !matches!(to, Status::PendingCustomer);
    if let (true, Some(paused_at), Some(calendar)) =
        (resumes, before.sla_paused_at, policy.calendar())
    {
        let paused = before.sla_paused_minutes + calendar.between(paused_at, at);
        row.sla_paused_minutes = Set(paused);
        row.sla_paused_at = Set(None);
        let priority = Priority::parse(&before.priority).unwrap_or(Priority::Normal);
        let (_, resolve) = due_dates(policy, before.created_at, priority, paused);
        row.resolve_due_at = Set(resolve);
    }
}

impl SupportService {
    /// Đánh dấu quá hạn mọi phiếu của khách `tenant` đã trễ hạn (một lần cho mỗi phiếu), phát
    /// `support.ticket_breached`. Trả số phiếu vừa đánh dấu.
    pub(crate) async fn mark_breached(
        &self,
        tenant: &TenantContext,
        at: DateTime<Utc>,
    ) -> Result<u64, Error> {
        let late = Condition::any()
            .add(
                Condition::all()
                    .add(ticket::Column::FirstRespondedAt.is_null())
                    .add(ticket::Column::FirstResponseDueAt.lt(at)),
            )
            .add(
                Condition::all()
                    .add(ticket::Column::SlaPausedAt.is_null())
                    .add(ticket::Column::ResolveDueAt.lt(at)),
            );
        let rows = ticket::Entity::find()
            .filter(ticket::Column::Status.is_in(ACTIVE))
            .filter(ticket::Column::Breached.eq(false))
            .filter(late)
            .scoped(tenant)
            .limit(BATCH)
            .all(tenant.database().read())
            .await?;
        let mut marked = 0;
        for before in rows {
            let transaction = tenant.database().begin_write().await?;
            let updated = ticket::Entity::update_many()
                .col_expr(ticket::Column::Breached, sea_orm::sea_query::Expr::value(true))
                .filter(ticket::Column::Id.eq(before.id))
                .filter(ticket::Column::Breached.eq(false))
                .scoped(tenant)
                .exec(&transaction)
                .await?;
            if updated.rows_affected == 0 {
                transaction.rollback().await?;
                continue;
            }
            let mut after = before.clone();
            after.breached = true;
            let event = events::TicketBreached(Change {
                before: Some(snapshot(&before)),
                after: Some(snapshot(&after)),
            });
            let subject = events::ticket_subject(before.id);
            self.parts
                .outbox
                .record(&transaction, tenant, Actor::system(), subject, &event)
                .await?;
            transaction.commit().await?;
            marked += 1;
        }
        Ok(marked)
    }

    /// Đóng phiếu `solved` không ai viết thêm sau `auto_close_days` ngày của chính sách nút.
    pub(crate) async fn auto_close(
        &self,
        tenant: &TenantContext,
        at: DateTime<Utc>,
    ) -> Result<u64, Error> {
        let rows = ticket::Entity::find()
            .filter(ticket::Column::Status.eq(Status::Solved.as_str()))
            .filter(ticket::Column::SolvedAt.lt(at - chrono::Duration::days(1)))
            .scoped(tenant)
            .limit(BATCH)
            .all(tenant.database().read())
            .await?;
        let mut policies: HashMap<Uuid, Policy> = HashMap::new();
        let mut closed = 0;
        for before in rows {
            let policy = match policies.get(&before.org_id) {
                Some(policy) => policy.clone(),
                None => {
                    let policy = self.policy(tenant, before.org_id).await?.policy;
                    policies.insert(before.org_id, policy.clone());
                    policy
                }
            };
            let due = before
                .solved_at
                .map(|solved| solved + chrono::Duration::days(i64::from(policy.auto_close_days)));
            if due.is_none_or(|due| due > at) {
                continue;
            }
            let transaction = tenant.database().begin_write().await?;
            let mut row = before.clone().into_active_model();
            transition(&mut row, &before, Status::Closed, None, &policy, at);
            row.updated_at = Set(at);
            let update = ticket::Entity::update(row)
                .validate()?
                .scoped(tenant)
                .filter(ticket::Column::Status.eq(Status::Solved.as_str()))
                .filter(ticket::Column::UpdatedAt.eq(before.updated_at));
            let after = match update.exec(&transaction).await {
                Ok(after) => after,
                // Người mua vừa viết thêm: để lượt sau.
                Err(sea_orm::DbErr::RecordNotUpdated) => {
                    transaction.rollback().await?;
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            self.system_line(&transaction, tenant, before.id, "status:closed", at).await?;
            self.record_changes(&transaction, tenant, Actor::system(), &before, &after, false)
                .await?;
            transaction.commit().await?;
            closed += 1;
        }
        Ok(closed)
    }
}

/// Job của một database: chạy `work` cho từng khách có phiếu còn mở trên database đó.
pub(crate) struct Sweep {
    pub service: Arc<SupportService>,
    pub databases: DatabaseRegistry,
    pub kind: &'static str,
}

#[async_trait::async_trait]
impl JobHandler for Sweep {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let statuses: Vec<&str> =
            if self.kind == AUTO_CLOSE { vec![Status::Solved.as_str()] } else { ACTIVE.to_vec() };
        let tenants: Vec<Uuid> = ticket::Entity::find()
            .select_only()
            .column(ticket::Column::TenantId)
            .distinct()
            .filter(ticket::Column::Status.is_in(statuses))
            .into_tuple()
            .all(job.database.read())
            .await?;
        let at = now();
        for id in tenants {
            let tenant = match self.databases.tenant(TenantId::from_uuid(id)).await {
                Ok(tenant) => tenant,
                Err(error) => {
                    tracing::warn!(tenant_id = %id, code = error.code(), "support sweep skipped a tenant");
                    continue;
                }
            };
            let done = if self.kind == AUTO_CLOSE {
                self.service.auto_close(&tenant, at).await?
            } else {
                self.service.mark_breached(&tenant, at).await?
            };
            if done > 0 {
                tracing::info!(tenant_id = %id, job = self.kind, tickets = done, "support sweep");
            }
        }
        Ok(())
    }
}

/// `support.tickets`: phiếu đang chờ nhân viên (mới hay người mua vừa viết) trong phạm vi người
/// gọi, hạn gần nhất trước — nguồn việc của hộp việc (bước 51).
pub(crate) struct Waiting;

impl Waiting {
    fn condition(principal: &Principal) -> Result<Condition, Error> {
        let scope = principal.scope_condition(
            &TICKETS_READ,
            ticket::Column::OrgId,
            Some(ticket::Column::AssigneeUserId),
        )?;
        Ok(Condition::all().add(scope).add(
            ticket::Column::Status.is_in([Status::Open.as_str(), Status::PendingAgent.as_str()]),
        ))
    }
}

#[async_trait::async_trait]
impl WorkSource for Waiting {
    fn id(&self) -> &'static str {
        "support.tickets"
    }

    fn permission(&self) -> Permission {
        TICKETS_READ
    }

    async fn count(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        _: OrgTarget,
    ) -> Result<u64, Error> {
        Ok(ticket::Entity::find()
            .filter(Self::condition(principal)?)
            .scoped(tenant)
            .count(tenant.database().read())
            .await?)
    }

    async fn items(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        _: OrgTarget,
        limit: u16,
    ) -> Result<Vec<WorkItem>, Error> {
        // Cũ nhất trước: người mua chờ lâu nhất.
        let page = PageRequest::first(u32::from(limit), Order::Asc);
        let select = ticket::Entity::find().filter(Self::condition(principal)?).scoped(tenant);
        let rows =
            fetch_page(select, ticket::Column::Id, |row| row.id, &page, tenant.database().read())
                .await?
                .data;
        Ok(rows
            .into_iter()
            .map(|row| {
                let severity = if row.breached { Severity::Blocking } else { Severity::Attention };
                WorkItem::new(
                    row.id.to_string(),
                    severity,
                    "Answer support ticket {number}",
                    "support.tickets",
                )
                .param("number", row.number)
                .due_at(
                    row.first_responded_at
                        .map_or(row.first_response_due_at, |_| row.resolve_due_at),
                )
            })
            .collect())
    }
}
