//! Hẹn giờ đăng / gỡ (D-102). Lịch là tài nguyên riêng, không phải một trạng thái của mục.
//!
//! - Tạo lịch đăng thì ghim một revision `scheduled` của bản nháp lúc đó: lúc chạy đăng đúng
//!   bản ấy, dù nháp đã sửa tiếp.
//! - Mỗi mục tối đa một lịch đăng và một lịch gỡ đang chờ (khoá dòng mục khi tạo, nên hai
//!   request cùng lúc không cùng qua).
//! - Lịch chạy bằng một job của kernel đúng `run_at`. Job nhận một lịch đang chờ bằng câu
//!   `UPDATE … WHERE status = 'scheduled'` trong transaction: chạy lại (thử lại, hai worker)
//!   không đăng hai lần.
//! - Bản công khai đã được đăng SAU lúc đặt lịch thì lịch thất bại với
//!   `content.schedule_outdated`, không ghi đè.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::{Actor, Outbox};
use autoxide_kernel::job::{EnqueueOptions, Job, JobContext, JobHandler};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::entity::{entry, published, schedule};
use crate::entries::revisions::{self, Reason, Snapshot};
use crate::entries::{Entry, Status, lock};
use crate::events::{self, Change};
use crate::permissions::{CONTENT_PUBLISH, CONTENT_READ};
use crate::publishing::paths;
use crate::service::{ContentService, entry_not_found, invalid, now};

pub(crate) const RUN: &str = "content.run_schedule";

/// Một lịch.
pub(crate) type Schedule = schedule::Model;

/// What a schedule does: `publish` or `unpublish`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Action {
    /// Publish the draft as it was when the schedule was made.
    Publish,
    /// Take the entry down.
    Unpublish,
}

impl Action {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Action::Publish => "publish",
            Action::Unpublish => "unpublish",
        }
    }
}

/// Một lịch sắp đặt: làm gì, lúc nào, người đặt có xác nhận đã duyệt nội dung do AI tạo không.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SchedulePlan {
    pub action: Action,
    pub run_at: Timestamp,
    pub ai_reviewed: bool,
}

/// Job: chạy lịch `schedule_id`.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct RunSchedule {
    pub schedule_id: Uuid,
}

impl Job for RunSchedule {
    const KIND: &'static str = RUN;
}

pub(crate) struct Runner(pub Arc<ContentService>);

#[async_trait::async_trait]
impl JobHandler for Runner {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        // Lịch xếp job cho khách của nó: lịch chạy (và event ghi) đúng khách đó.
        let tenant = job
            .tenant
            .as_ref()
            .ok_or_else(|| Error::internal("a content schedule job must belong to a tenant"))?;
        let input: RunSchedule = serde_json::from_value(job.payload.clone())
            .map_err(|error| Error::internal(format!("content schedule job payload: {error}")))?;
        self.0.run_schedule(tenant, input.schedule_id, now()).await
    }
}

impl ContentService {
    /// Đặt lịch `action` cho mục `entry_id` lúc `run_at` (phải ở tương lai). Cần
    /// `content.publish`. `expected`: phiên bản mục đã đọc (`If-Match`).
    #[cfg(test)]
    pub(crate) async fn create_schedule(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        entry_id: Uuid,
        action: Action,
        run_at: Timestamp,
        expected: Option<Timestamp>,
    ) -> Result<Schedule, Error> {
        let plan = SchedulePlan { action, run_at, ai_reviewed: false };
        self.create_schedule_reviewed(tenant, principal, entry_id, plan, expected).await
    }

    /// Như [`create_schedule`](Self::create_schedule); `ai_reviewed`: người đặt lịch xác nhận đã
    /// duyệt nội dung do AI tạo (như `ai_reviewed` khi đăng ngay) — chỉ người đăng nhập, không
    /// phải API key; lịch chạy thì bản công khai mang xác nhận đó.
    pub(crate) async fn create_schedule_reviewed(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        entry_id: Uuid,
        plan: SchedulePlan,
        expected: Option<Timestamp>,
    ) -> Result<Schedule, Error> {
        let SchedulePlan { action, run_at, ai_reviewed } = plan;
        principal.require(&CONTENT_PUBLISH)?;
        if ai_reviewed && !principal.is_session() {
            return Err(Error::forbidden(
                "content.ai_review_requires_human",
                "Only a person signed in (not an API key) can confirm that AI-generated content was reviewed.",
            ));
        }
        let entry = self.load_checked(tenant, entry_id, expected).await?;
        crate::permissions::require_org(
            principal,
            tenant,
            &entry,
            crate::permissions::Need::Publish,
        )
        .await?;
        let now = now();
        if run_at <= now {
            return Err(invalid(vec![FieldError::new(
                "run_at",
                "schedule_in_past",
                "must be in the future",
            )]));
        }
        if action == Action::Publish {
            // Ảnh của bản sẽ đăng phải công khai — báo ngay, không đợi tới giờ chạy.
            self.check_publishable(tenant, &entry).await?;
        }
        let by = Some(principal.user_id());
        let transaction = tenant.database().begin_write().await?;
        // Khoá dòng mục: hai request đặt lịch cùng lúc chạy lần lượt.
        if !lock(&transaction, tenant, entry_id).await? {
            return Err(entry_not_found(entry_id));
        }
        let current = self
            .reload(tenant, &transaction, entry_id, expected.unwrap_or(entry.updated_at))
            .await?;
        let pending = schedule::Entity::find()
            .scoped(tenant)
            .filter(schedule::Column::EntryId.eq(entry_id))
            .filter(schedule::Column::Action.eq(action.as_str()))
            .filter(schedule::Column::Status.eq("scheduled"))
            .count(&transaction)
            .await?;
        if pending > 0 {
            return Err(Error::conflict(
                "content.schedule_conflict",
                "This entry already has a pending `{action}` schedule. Cancel it first.",
            )
            .with_param("action", action.as_str()));
        }
        let revision_id = match action {
            Action::Publish => Some(
                revisions::record(&transaction, tenant, &current, Reason::Scheduled, by, now)
                    .await?
                    .id,
            ),
            Action::Unpublish => None,
        };
        let row = schedule::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            entry_id: Set(entry_id),
            action: Set(action.as_str().to_owned()),
            revision_id: Set(revision_id),
            run_at: Set(run_at),
            status: Set("scheduled".to_owned()),
            error_code: Set(None),
            created_by: Set(by),
            ai_reviewed_by: Set((ai_reviewed && action == Action::Publish && current.ai_generated)
                .then_some(principal.user_id())),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&transaction)
        .await?;
        let job = RunSchedule { schedule_id: row.id };
        let options = EnqueueOptions::default().run_at(run_at);
        self.jobs.enqueue(&transaction, tenant, &job, options).await?;
        let change = Change { before: None, after: Some(schedule_summary(&row)), published: None };
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                events::entry_subject(entry_id),
                &events::ScheduleCreated(change),
            )
            .await?;
        transaction.commit().await?;
        Ok(row)
    }

    /// Lịch của mọi mục, theo giờ chạy (`order=asc`: sớm nhất trước), lọc theo `status` (vd
    /// `scheduled`: đang chờ) — màn "Sắp đăng" (đề bài giao diện B4). Cần `content.read`.
    pub(crate) async fn list_all_schedules(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        status: Option<&str>,
        page: &autoxide_kernel::pagination::PageRequest,
    ) -> Result<autoxide_kernel::pagination::Page<Schedule>, Error> {
        principal.require(&CONTENT_READ)?;
        // Chỉ lịch của mục trong phạm vi `content.read` (bước 34).
        let scope = principal.scope_condition(
            &CONTENT_READ,
            crate::entity::entry::Column::OrgId,
            Some(crate::entity::entry::Column::CreatedBy),
        )?;
        let permitted = sea_orm::sea_query::Query::select()
            .column(crate::entity::entry::Column::Id)
            .from(crate::entity::entry::Entity)
            .cond_where(scope)
            .and_where(crate::entity::entry::Column::TenantId.eq(tenant.id().as_uuid()))
            .to_owned();
        let mut select = schedule::Entity::find()
            .filter(schedule::Column::EntryId.in_subquery(permitted))
            .scoped(tenant);
        if let Some(status) = status {
            select = select.filter(schedule::Column::Status.eq(status));
        }
        let key = |row: &Schedule| autoxide_kernel::pagination::SortValue::Time(row.run_at);
        autoxide_kernel::pagination::fetch_page_by(
            select,
            schedule::Column::RunAt,
            key,
            schedule::Column::Id,
            |row| row.id,
            page,
            tenant.database().read(),
        )
        .await
    }

    /// Một trang lịch của mục `entry_id`, mới trước (theo id), keyset trong database (index
    /// `content_schedules_entry_page_idx`). Cần `content.read`.
    pub(crate) async fn list_schedules(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        entry_id: Uuid,
        page: &autoxide_kernel::pagination::PageRequest,
    ) -> Result<autoxide_kernel::pagination::Page<Schedule>, Error> {
        principal.require(&CONTENT_READ)?;
        self.get(tenant, principal, entry_id).await?;
        let select =
            schedule::Entity::find().scoped(tenant).filter(schedule::Column::EntryId.eq(entry_id));
        autoxide_kernel::pagination::fetch_page(
            select,
            schedule::Column::Id,
            |row| row.id,
            page,
            tenant.database().read(),
        )
        .await
    }

    /// Huỷ lịch `id` đang chờ. Cần `content.publish`.
    pub(crate) async fn cancel_schedule(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Schedule, Error> {
        principal.require(&CONTENT_PUBLISH)?;
        let before = schedule::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| schedule_not_found(id))?;
        // Lịch của mục ngoài phạm vi theo nút: như không có (bước 34).
        let entry = crate::entity::entry::Entity::find_by_id(before.entry_id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| schedule_not_found(id))?;
        crate::permissions::require_org(
            principal,
            tenant,
            &entry,
            crate::permissions::Need::Publish,
        )
        .await
        .map_err(|error| {
            if error.code() == "content.not_found" { schedule_not_found(id) } else { error }
        })?;
        let transaction = tenant.database().begin_write().await?;
        let canceled = set_schedule_status(tenant, &transaction, id, "canceled", None).await?;
        let Some(after) = canceled else {
            return Err(Error::conflict(
                "content.schedule_not_pending",
                "The schedule already ran or was canceled.",
            ));
        };
        let change = Change {
            before: Some(schedule_summary(&before)),
            after: Some(schedule_summary(&after)),
            published: None,
        };
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                events::entry_subject(before.entry_id),
                &events::ScheduleCanceled(change),
            )
            .await?;
        transaction.commit().await?;
        Ok(after)
    }

    /// Chạy lịch `id` (job). Lịch không còn chờ thì không làm gì. Lỗi nghiệp vụ (đường dẫn
    /// trùng, bản công khai mới hơn…) làm lịch `failed`, không thử lại; lỗi khác (database)
    /// trả ra để job thử lại.
    pub(crate) async fn run_schedule(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        now: Timestamp,
    ) -> Result<(), Error> {
        let Some(row) =
            schedule::Entity::find_by_id(id).scoped(tenant).one(tenant.database().write()).await?
        else {
            return Ok(());
        };
        if row.status != "scheduled" {
            return Ok(());
        }
        match self.execute(tenant, &row, now).await {
            Ok(()) => Ok(()),
            Err(error)
                if error.code().starts_with("content.") || error.code() == "validation_failed" =>
            {
                let code = if error.code() == "validation_failed" {
                    "content.invalid_body"
                } else {
                    error.code()
                };
                tracing::warn!(schedule_id = %id, code, "content schedule failed");
                let transaction = tenant.database().begin_write().await?;
                if let Some(after) =
                    set_schedule_status(tenant, &transaction, id, "failed", Some(code)).await?
                {
                    let change = Change {
                        before: Some(schedule_summary(&row)),
                        after: Some(schedule_summary(&after)),
                        published: None,
                    };
                    self.outbox
                        .record(
                            &transaction,
                            tenant,
                            row.created_by.map_or_else(Actor::system, Actor::user),
                            events::entry_subject(row.entry_id),
                            &events::ScheduleFailed(change),
                        )
                        .await?;
                }
                transaction.commit().await?;
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    async fn execute(
        &self,
        tenant: &TenantContext,
        row: &Schedule,
        now: Timestamp,
    ) -> Result<(), Error> {
        let entry = entry::Entity::find_by_id(row.entry_id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| entry_not_found(row.entry_id))?;
        // Bản sẽ đăng: bản nháp lúc đặt lịch (revision đã ghim).
        let version = match (row.action.as_str(), row.revision_id) {
            ("publish", Some(revision_id)) => {
                let revision =
                    revisions::revision_entity(tenant.database().read(), tenant, revision_id)
                        .await?;
                let snapshot: Snapshot =
                    serde_json::from_str(&revision.snapshot).map_err(Error::internal)?;
                let mut version = entry.clone();
                let mut problems = Vec::new();
                self.apply(tenant, &mut version, revisions::fields_of(snapshot), &mut problems)
                    .await?;
                if !problems.is_empty() {
                    return Err(invalid(problems));
                }
                self.check_publishable(tenant, &version).await?;
                Some((version, revision_id))
            }
            _ => None,
        };
        let transaction = tenant.database().begin_write().await?;
        paths::lock_tree(&transaction, tenant, &entry.locale).await?;
        // Nhận lịch: chỉ một lần chạy qua được câu này.
        if set_schedule_status(tenant, &transaction, row.id, "succeeded", None).await?.is_none() {
            return Ok(());
        }
        let current = entry::Entity::find_by_id(row.entry_id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| entry_not_found(row.entry_id))?;
        let actor = row.created_by.map_or_else(Actor::system, Actor::user);
        match version {
            Some((mut version, revision_id)) => {
                // Nội dung lấy từ revision đã ghim; còn vị trí trong cây lấy từ bản đọc SAU khi
                // khoá: một lệnh `move` có thể vừa xong giữa lúc đọc `entry` và lúc khoá.
                version.parent_id = current.parent_id;
                version.parent_key = current.parent_key;
                if let Some(live) = published::Entity::find()
                    .scoped(tenant)
                    .filter(published::Column::EntryId.eq(row.entry_id))
                    .one(&transaction)
                    .await?
                    && live.content_modified_at > row.created_at
                {
                    return Err(Error::conflict(
                        "content.schedule_outdated",
                        "The entry was published after this schedule was made; it was not overwritten.",
                    ));
                }
                let (path, moved) = self
                    .put_published(
                        tenant,
                        &transaction,
                        &version,
                        revision_id,
                        crate::publishing::PublishStamp {
                            published_at: None,
                            now,
                            by: row.created_by,
                            ai_reviewed_by: row.ai_reviewed_by,
                        },
                    )
                    .await?;
                let mut after = current.clone();
                after.status = Status::Published.as_str().to_owned();
                after.has_unpublished_changes = differs(&current, &version);
                after.updated_at = now;
                let after = self.save_state(tenant, &transaction, &current, &after).await?;
                let change = Change {
                    before: Some(crate::entries::summary(&current)),
                    after: Some(crate::entries::summary(&after)),
                    published: Some(crate::publishing::published_payload(
                        &after,
                        Some(&path),
                        &moved,
                    )),
                };
                self.enqueue_index(tenant, &transaction, after.id).await?;
                self.outbox
                    .record(
                        &transaction,
                        tenant,
                        actor,
                        events::entry_subject(after.id),
                        &events::EntryPublished(change),
                    )
                    .await?;
            }
            None => {
                if Status::of(&current) != Status::Published {
                    return Err(Error::conflict(
                        "content.not_published",
                        "The entry is not published.",
                    ));
                }
                let removed = self
                    .remove_published(tenant, &transaction, &current, false, row.created_by, now)
                    .await?
                    .unwrap_or_default();
                // Như gỡ bằng tay: ghi revision `unpublished`.
                revisions::record(
                    &transaction,
                    tenant,
                    &current,
                    Reason::Unpublished,
                    row.created_by,
                    now,
                )
                .await?;
                let mut after = current.clone();
                after.status = Status::Draft.as_str().to_owned();
                after.has_unpublished_changes = false;
                after.updated_at = now;
                let after = self.save_state(tenant, &transaction, &current, &after).await?;
                let stale = vec![(after.id, removed, String::new())];
                let change = Change {
                    before: Some(crate::entries::summary(&current)),
                    after: Some(crate::entries::summary(&after)),
                    published: Some(crate::publishing::published_payload(&after, None, &stale)),
                };
                self.enqueue_index(tenant, &transaction, after.id).await?;
                self.outbox
                    .record(
                        &transaction,
                        tenant,
                        actor,
                        events::entry_subject(after.id),
                        &events::EntryUnpublished(change),
                    )
                    .await?;
            }
        }
        transaction.commit().await?;
        Ok(())
    }

    /// Huỷ các lịch `actions` đang chờ của mục `entry_id`, trong `transaction` của thao tác gây
    /// ra (gỡ, lưu trữ), mỗi lịch kèm event `content.schedule_canceled`. Không huỷ thì một lịch
    /// đăng đặt từ trước sẽ đăng lại mục vừa bị gỡ hay lưu trữ. Trả số lịch đã huỷ.
    pub(crate) async fn cancel_pending_schedules<C: sea_orm::ConnectionTrait>(
        &self,
        tenant: &TenantContext,
        transaction: &C,
        actor: Actor,
        entry_id: Uuid,
        actions: &[Action],
    ) -> Result<u64, Error> {
        let pending = schedule::Entity::find()
            .scoped(tenant)
            .filter(schedule::Column::EntryId.eq(entry_id))
            .filter(schedule::Column::Status.eq("scheduled"))
            .filter(schedule::Column::Action.is_in(actions.iter().map(|action| action.as_str())))
            .all(transaction)
            .await?;
        let mut canceled = 0;
        for before in pending {
            let Some(after) =
                set_schedule_status(tenant, transaction, before.id, "canceled", None).await?
            else {
                continue;
            };
            let change = Change {
                before: Some(schedule_summary(&before)),
                after: Some(schedule_summary(&after)),
                published: None,
            };
            self.outbox
                .record(
                    transaction,
                    tenant,
                    actor,
                    events::entry_subject(entry_id),
                    &events::ScheduleCanceled(change),
                )
                .await?;
            canceled += 1;
        }
        Ok(canceled)
    }
}

/// Huỷ mọi lịch đang chờ do người `user_id` đặt; trả số lịch đã huỷ. Mỗi lịch kèm event
/// `content.schedule_canceled` qua `outbox` — `None` khi module đang tắt (chế độ nghỉ, bước 70 P9):
/// không ai phát event của module tắt, nên không ghi.
pub(crate) async fn cancel_schedules_by(
    tenant: &TenantContext,
    outbox: Option<&Outbox>,
    user_id: Uuid,
) -> Result<u64, Error> {
    let pending = schedule::Entity::find()
        .scoped(tenant)
        .filter(schedule::Column::CreatedBy.eq(user_id))
        .filter(schedule::Column::Status.eq("scheduled"))
        .all(tenant.database().read())
        .await?;
    let mut canceled = 0;
    // Mỗi lịch một transaction ngắn, kèm event như huỷ bằng tay (audit, webhook). Lịch vừa
    // chạy hay vừa bị huỷ ở nơi khác thì bỏ qua: nhận lại event xoá tài khoản cũng an toàn.
    for before in pending {
        let transaction = tenant.database().begin_write().await?;
        let Some(after) =
            set_schedule_status(tenant, &transaction, before.id, "canceled", None).await?
        else {
            transaction.rollback().await?;
            continue;
        };
        if let Some(outbox) = outbox {
            let change = Change {
                before: Some(schedule_summary(&before)),
                after: Some(schedule_summary(&after)),
                published: None,
            };
            outbox
                .record(
                    &transaction,
                    tenant,
                    Actor::system(),
                    events::entry_subject(before.entry_id),
                    &events::ScheduleCanceled(change),
                )
                .await?;
        }
        transaction.commit().await?;
        canceled += 1;
    }
    Ok(canceled)
}

/// Đổi trạng thái lịch `id` từ `scheduled` sang `status`; `None` nếu lịch không còn chờ.
async fn set_schedule_status<C: sea_orm::ConnectionTrait>(
    tenant: &TenantContext,
    connection: &C,
    id: Uuid,
    status: &str,
    error: Option<&str>,
) -> Result<Option<Schedule>, Error> {
    let updated = schedule::Entity::update_many()
        .scoped(tenant)
        .col_expr(schedule::Column::Status, Expr::value(status))
        .col_expr(schedule::Column::ErrorCode, Expr::value(error.map(str::to_owned)))
        .col_expr(schedule::Column::UpdatedAt, Expr::value(now()))
        .filter(schedule::Column::Id.eq(id))
        .filter(schedule::Column::Status.eq("scheduled"))
        .exec(connection)
        .await?;
    if updated.rows_affected == 0 {
        return Ok(None);
    }
    Ok(schedule::Entity::find_by_id(id).scoped(tenant).one(connection).await?)
}

/// Bản nháp `draft` có khác phiên bản vừa đăng `version` không.
fn differs(draft: &Entry, version: &Entry) -> bool {
    revisions::Snapshot::of(draft) != revisions::Snapshot::of(version)
}

fn schedule_summary(row: &Schedule) -> serde_json::Value {
    json!({
        "schedule_id": row.id,
        "action": row.action,
        "run_at": row.run_at,
        "status": row.status,
        "error_code": row.error_code,
        "revision_id": row.revision_id,
    })
}

pub(crate) fn schedule_not_found(id: Uuid) -> Error {
    Error::not_found("content.schedule_not_found", "No schedule has the id `{id}`.")
        .with_param("id", id)
}
