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

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::{EnqueueOptions, Job, JobContext, JobHandler};
use chrono::{DateTime, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, Set,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::entity::{entry, published, schedule};
use crate::entries::{Entry, Status, touch};
use crate::events::{self, Change};
use crate::paths;
use crate::permissions::{CONTENT_PUBLISH, CONTENT_READ};
use crate::revisions::{self, Reason, Snapshot};
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
        let input: RunSchedule = serde_json::from_value(job.payload.clone())
            .map_err(|error| Error::internal(format!("content schedule job payload: {error}")))?;
        self.0.run_schedule(input.schedule_id, now()).await
    }
}

impl ContentService {
    /// Đặt lịch `action` cho mục `entry_id` lúc `run_at` (phải ở tương lai). Cần
    /// `content.publish`. `expected`: phiên bản mục đã đọc (`If-Match`).
    pub(crate) async fn create_schedule(
        &self,
        principal: &Principal,
        entry_id: Uuid,
        action: Action,
        run_at: DateTime<Utc>,
        expected: Option<DateTime<Utc>>,
    ) -> Result<Schedule, Error> {
        principal.require(&CONTENT_PUBLISH)?;
        let entry = self.load_checked(entry_id, expected).await?;
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
            self.check_publishable(&entry).await?;
        }
        let by = Some(principal.user_id());
        let transaction = self.database.begin_write().await?;
        // Khoá dòng mục: hai request đặt lịch cùng lúc chạy lần lượt.
        if !touch(&transaction, entry_id).await? {
            return Err(entry_not_found(entry_id));
        }
        let current =
            self.reload(&transaction, entry_id, expected.unwrap_or(entry.updated_at)).await?;
        let pending = schedule::Entity::find()
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
                revisions::record(&transaction, &current, Reason::Scheduled, by, now).await?.id,
            ),
            Action::Unpublish => None,
        };
        let row = schedule::ActiveModel {
            id: Set(Uuid::now_v7()),
            entry_id: Set(entry_id),
            action: Set(action.as_str().to_owned()),
            revision_id: Set(revision_id),
            run_at: Set(run_at),
            status: Set("scheduled".to_owned()),
            error_code: Set(None),
            created_by: Set(by),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&transaction)
        .await?;
        let job = RunSchedule { schedule_id: row.id };
        let options = EnqueueOptions { run_at: Some(run_at), ..Default::default() };
        self.jobs.enqueue(&transaction, &job, options).await?;
        let change = Change { before: None, after: Some(schedule_summary(&row)), published: None };
        self.outbox
            .record(
                &transaction,
                Actor::from(principal),
                events::entry_subject(entry_id),
                &events::ScheduleCreated(change),
            )
            .await?;
        transaction.commit().await?;
        Ok(row)
    }

    /// Lịch của mục `entry_id`, mới trước. Cần `content.read`.
    pub(crate) async fn list_schedules(
        &self,
        principal: &Principal,
        entry_id: Uuid,
    ) -> Result<Vec<Schedule>, Error> {
        principal.require(&CONTENT_READ)?;
        Ok(schedule::Entity::find()
            .filter(schedule::Column::EntryId.eq(entry_id))
            .order_by_desc(schedule::Column::Id)
            .all(self.database.read())
            .await?)
    }

    /// Huỷ lịch `id` đang chờ. Cần `content.publish`.
    pub(crate) async fn cancel_schedule(
        &self,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Schedule, Error> {
        principal.require(&CONTENT_PUBLISH)?;
        let before = schedule::Entity::find_by_id(id)
            .one(self.database.read())
            .await?
            .ok_or_else(|| schedule_not_found(id))?;
        let transaction = self.database.begin_write().await?;
        let canceled = self.set_schedule_status(&transaction, id, "canceled", None).await?;
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
    pub(crate) async fn run_schedule(&self, id: Uuid, now: DateTime<Utc>) -> Result<(), Error> {
        let Some(row) = schedule::Entity::find_by_id(id).one(self.database.write()).await? else {
            return Ok(());
        };
        if row.status != "scheduled" {
            return Ok(());
        }
        match self.execute(&row, now).await {
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
                let transaction = self.database.begin_write().await?;
                if let Some(after) =
                    self.set_schedule_status(&transaction, id, "failed", Some(code)).await?
                {
                    let change = Change {
                        before: Some(schedule_summary(&row)),
                        after: Some(schedule_summary(&after)),
                        published: None,
                    };
                    self.outbox
                        .record(
                            &transaction,
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

    async fn execute(&self, row: &Schedule, now: DateTime<Utc>) -> Result<(), Error> {
        let entry = entry::Entity::find_by_id(row.entry_id)
            .one(self.database.read())
            .await?
            .ok_or_else(|| entry_not_found(row.entry_id))?;
        // Bản sẽ đăng: bản nháp lúc đặt lịch (revision đã ghim).
        let version = match (row.action.as_str(), row.revision_id) {
            ("publish", Some(revision_id)) => {
                let revision =
                    revisions::revision_entity(self.database.read(), revision_id).await?;
                let snapshot: Snapshot =
                    serde_json::from_str(&revision.snapshot).map_err(Error::internal)?;
                let mut version = entry.clone();
                let mut problems = Vec::new();
                self.apply(&mut version, revisions::fields_of(snapshot), &mut problems).await?;
                if !problems.is_empty() {
                    return Err(invalid(problems));
                }
                self.check_publishable(&version).await?;
                Some((version, revision_id))
            }
            _ => None,
        };
        let transaction = self.database.begin_write().await?;
        paths::lock_tree(&transaction, &entry.locale).await?;
        // Nhận lịch: chỉ một lần chạy qua được câu này.
        if self.set_schedule_status(&transaction, row.id, "succeeded", None).await?.is_none() {
            return Ok(());
        }
        let current = entry::Entity::find_by_id(row.entry_id)
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
                if let Some(live) =
                    published::Entity::find_by_id(row.entry_id).one(&transaction).await?
                    && live.content_modified_at > row.created_at
                {
                    return Err(Error::conflict(
                        "content.schedule_outdated",
                        "The entry was published after this schedule was made; it was not overwritten.",
                    ));
                }
                let (path, moved) = self
                    .put_published(
                        &transaction,
                        &version,
                        revision_id,
                        crate::publishing::PublishStamp {
                            published_at: None,
                            now,
                            by: row.created_by,
                            ai_reviewed_by: None,
                        },
                    )
                    .await?;
                let mut after = current.clone();
                after.status = Status::Published.as_str().to_owned();
                after.has_unpublished_changes = differs(&current, &version);
                after.updated_at = now;
                let after = self.save_state(&transaction, &current, &after).await?;
                let change = Change {
                    before: Some(crate::entries::summary(&current)),
                    after: Some(crate::entries::summary(&after)),
                    published: Some(crate::publishing::published_payload(
                        &after,
                        Some(&path),
                        &moved,
                    )),
                };
                self.outbox
                    .record(
                        &transaction,
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
                    .remove_published(&transaction, &current, false, row.created_by, now)
                    .await?
                    .unwrap_or_default();
                // Như gỡ bằng tay: ghi revision `unpublished`.
                revisions::record(&transaction, &current, Reason::Unpublished, row.created_by, now)
                    .await?;
                let mut after = current.clone();
                after.status = Status::Draft.as_str().to_owned();
                after.has_unpublished_changes = false;
                after.updated_at = now;
                let after = self.save_state(&transaction, &current, &after).await?;
                let stale = vec![(after.id, removed, String::new())];
                let change = Change {
                    before: Some(crate::entries::summary(&current)),
                    after: Some(crate::entries::summary(&after)),
                    published: Some(crate::publishing::published_payload(&after, None, &stale)),
                };
                self.outbox
                    .record(
                        &transaction,
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

    /// Đổi trạng thái lịch `id` từ `scheduled` sang `status`; `None` nếu lịch không còn chờ.
    async fn set_schedule_status<C: sea_orm::ConnectionTrait>(
        &self,
        connection: &C,
        id: Uuid,
        status: &str,
        error: Option<&str>,
    ) -> Result<Option<Schedule>, Error> {
        let updated = schedule::Entity::update_many()
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
        Ok(schedule::Entity::find_by_id(id).one(connection).await?)
    }

    /// Huỷ mọi lịch đang chờ do người `user_id` đặt (tài khoản bị xoá). Trả số lịch đã huỷ.
    pub(crate) async fn cancel_schedules_of(&self, user_id: Uuid) -> Result<u64, Error> {
        let pending = schedule::Entity::find()
            .filter(schedule::Column::CreatedBy.eq(user_id))
            .filter(schedule::Column::Status.eq("scheduled"))
            .all(self.database.read())
            .await?;
        let mut canceled = 0;
        // Mỗi lịch một transaction ngắn, kèm event như huỷ bằng tay (audit, webhook). Lịch vừa
        // chạy hay vừa bị huỷ ở nơi khác thì bỏ qua: nhận lại event xoá tài khoản cũng an toàn.
        for before in pending {
            let transaction = self.database.begin_write().await?;
            let Some(after) =
                self.set_schedule_status(&transaction, before.id, "canceled", None).await?
            else {
                transaction.rollback().await?;
                continue;
            };
            let change = Change {
                before: Some(schedule_summary(&before)),
                after: Some(schedule_summary(&after)),
                published: None,
            };
            self.outbox
                .record(
                    &transaction,
                    Actor::system(),
                    events::entry_subject(before.entry_id),
                    &events::ScheduleCanceled(change),
                )
                .await?;
            transaction.commit().await?;
            canceled += 1;
        }
        Ok(canceled)
    }
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
