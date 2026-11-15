//! Đích của bình luận: cặp (`target_type`, `target_id`) — không foreign key, không đọc bảng của
//! module sở hữu. Loại đích phải nằm trong `AUTOXIDE_COMMENTS_TARGET_TYPES`; loại có bộ tra trong
//! sổ đối tượng của kernel (D-123, vd `content.entry`) thì đích phải có và công khai.
//!
//! Mở / đóng / ẩn theo đích (quản trị viên, tool MCP), tự đóng sau `closes_at`, và nghe event
//! vòng đời mà module sở hữu khai qua `Module::resource_events` (D-202): `Closed` → đóng,
//! `Removed` → đóng và ẩn. `comments` không viết cứng tên event của module nào. Số đếm nằm ở
//! `comments_targets`, đếm theo lô tối đa 100 đích.

use std::collections::HashMap;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::{Actor, Delivered, EventHandler};
use autoxide_kernel::resource::{Resource, ResourceChange, ResourceEvent};
use chrono::{DateTime, Utc};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, Set};
use serde_json::json;

use crate::entity::target;
use crate::events::{self, Change};
use crate::permissions::{COMMENTS_MANAGE, COMMENTS_READ};
use crate::service::{CommentsService, ensure_target, now};
use crate::text;

/// Số đích tối đa của một lần đếm.
pub(crate) const COUNTS_MAX: usize = 100;

/// Trạng thái của một đích, như service trả về.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Target {
    pub kind: String,
    pub id: String,
    /// Trạng thái đã đặt (`open` / `closed`).
    pub state: String,
    pub hidden: bool,
    pub closes_at: Option<DateTime<Utc>>,
    pub published_count: i32,
    pub pending_count: i32,
    /// Số bình luận gốc (câu hỏi) đang hiện / chờ duyệt.
    pub published_threads: i32,
    pub pending_threads: i32,
    /// `None` khi chưa có dòng nào (chưa có bình luận, chưa ai đặt trạng thái).
    pub updated_at: Option<DateTime<Utc>>,
}

impl Target {
    fn from_row(row: target::Model) -> Self {
        Target {
            kind: row.target_type,
            id: row.target_id,
            state: row.state,
            hidden: row.hidden,
            closes_at: row.closes_at,
            published_count: row.published_count,
            pending_count: row.pending_count,
            published_threads: row.published_thread_count,
            pending_threads: row.pending_thread_count,
            updated_at: Some(row.updated_at),
        }
    }

    fn blank(kind: &str, id: &str) -> Self {
        Target {
            kind: kind.to_owned(),
            id: id.to_owned(),
            state: "open".to_owned(),
            hidden: false,
            closes_at: None,
            published_count: 0,
            pending_count: 0,
            published_threads: 0,
            pending_threads: 0,
            updated_at: None,
        }
    }

    /// Có nhận bình luận mới lúc `now` không.
    pub(crate) fn is_open(&self, now: DateTime<Utc>) -> bool {
        self.state == "open" && !self.hidden && self.closes_at.is_none_or(|at| at > now)
    }

    /// Trạng thái thật lúc này: `closed` khi đã đóng, bị ẩn hay quá `closes_at`.
    pub(crate) fn effective_state(&self) -> &'static str {
        if self.is_open(now()) { "open" } else { "closed" }
    }
}

/// Thay đổi trạng thái đích do quản trị viên gửi.
#[derive(Debug, Clone, Default)]
pub(crate) struct TargetChange {
    /// `open` hoặc `closed`.
    pub state: Option<String>,
    pub hidden: Option<bool>,
    /// `Some(None)`: bỏ hạn tự đóng.
    pub closes_at: Option<Option<DateTime<Utc>>>,
}

pub(crate) fn target_not_found() -> Error {
    Error::not_found(
        "comments.target_not_found",
        "Comments are not taken on this target: its kind is not in \
         `AUTOXIDE_COMMENTS_TARGET_TYPES`, or it does not exist or is not public.",
    )
}

pub(crate) fn target_closed() -> Error {
    Error::conflict("comments.target_closed", "This target does not take new comments.")
}

impl CommentsService {
    /// Kiểm loại và dạng ID của đích (không tra module sở hữu). Loại lạ → 404.
    pub(crate) fn check_target(&self, kind: &str, id: &str) -> Result<(), Error> {
        if !self.config.target_types.iter().any(|known| known == kind) {
            return Err(target_not_found());
        }
        if !text::is_target_id(id) {
            return Err(Error::validation(vec![FieldError::new(
                "target_id",
                "invalid",
                "must be 1 to 100 letters, digits, `-`, `_`, `.` or `:`",
            )]));
        }
        Ok(())
    }

    /// Đích có thật và công khai không, khi loại của nó có bộ tra trong sổ đối tượng. Loại không
    /// có bộ tra được chấp nhận theo cấu hình (`None`). Gọi NGOÀI transaction.
    pub(crate) async fn resolve_target(
        &self,
        kind: &str,
        id: &str,
    ) -> Result<Option<Resource>, Error> {
        self.check_target(kind, id)?;
        if !self.resources.knows(kind) {
            return Ok(None);
        }
        match self.resources.resolve(kind, id).await? {
            Some(resource) if resource.public => Ok(Some(resource)),
            _ => Err(target_not_found()),
        }
    }

    /// Trạng thái của đích (mặc định khi chưa có dòng).
    pub(crate) async fn target(&self, kind: &str, id: &str) -> Result<Target, Error> {
        self.check_target(kind, id)?;
        let row = target::Entity::find_by_id((kind.to_owned(), id.to_owned()))
            .one(self.database.read())
            .await?;
        Ok(row.map_or_else(|| Target::blank(kind, id), Target::from_row))
    }

    /// Trạng thái của đích cho quản trị viên. Cần `comments.read`.
    pub(crate) async fn admin_target(
        &self,
        principal: &Principal,
        kind: &str,
        id: &str,
    ) -> Result<Target, Error> {
        principal.require(&COMMENTS_READ)?;
        self.target(kind, id).await
    }

    /// Số bình luận đang hiện của nhiều đích cùng loại (tối đa [`COUNTS_MAX`]), theo thứ tự
    /// hỏi: (ID, mọi bình luận, bình luận gốc); đích ẩn đếm 0.
    pub(crate) async fn counts(
        &self,
        kind: &str,
        ids: &[String],
    ) -> Result<Vec<(String, i32, i32)>, Error> {
        if ids.is_empty() || ids.len() > COUNTS_MAX {
            return Err(Error::validation(vec![FieldError::new(
                "target_ids",
                "length",
                format!("must list 1 to {COUNTS_MAX} ids, separated by commas"),
            )]));
        }
        for id in ids {
            self.check_target(kind, id)?;
        }
        let found: HashMap<String, (i32, i32)> = target::Entity::find()
            .filter(target::Column::TargetType.eq(kind))
            .filter(target::Column::TargetId.is_in(ids.to_vec()))
            .all(self.database.read())
            .await?
            .into_iter()
            .map(|row| {
                let counts = if row.hidden {
                    (0, 0)
                } else {
                    (row.published_count, row.published_thread_count)
                };
                (row.target_id, counts)
            })
            .collect();
        Ok(ids
            .iter()
            .map(|id| {
                let (all, threads) = found.get(id).copied().unwrap_or((0, 0));
                (id.clone(), all, threads)
            })
            .collect())
    }

    /// Mở / đóng / ẩn đích. Cần `comments.manage`. `expected`: phiên bản đã đọc (`If-Match`).
    pub(crate) async fn set_target(
        &self,
        principal: &Principal,
        kind: &str,
        id: &str,
        change: TargetChange,
        expected: Option<DateTime<Utc>>,
    ) -> Result<Target, Error> {
        principal.require(&COMMENTS_MANAGE)?;
        self.check_target(kind, id)?;
        if let Some(state) = change.state.as_deref()
            && state != "open"
            && state != "closed"
        {
            return Err(Error::validation(vec![FieldError::new(
                "state",
                "invalid",
                "must be `open` or `closed`",
            )]));
        }
        self.change_target(Actor::from(principal), kind, id, change, expected).await
    }

    /// Đổi trạng thái đích trong một transaction, kèm event khi trạng thái thật đổi.
    pub(crate) async fn change_target(
        &self,
        actor: Actor,
        kind: &str,
        id: &str,
        change: TargetChange,
        expected: Option<DateTime<Utc>>,
    ) -> Result<Target, Error> {
        let now = now();
        let transaction = self.database.begin_write().await?;
        let existing =
            target::Entity::find_by_id((kind.to_owned(), id.to_owned())).one(&transaction).await?;
        // Chưa có dòng thì phiên bản đã đọc phải là "chưa có" (`If-Match: *` hay không gửi).
        if let Some(expected) = expected
            && existing.as_ref().is_none_or(|row| row.updated_at != expected)
        {
            return Err(autoxide_kernel::version::changed());
        }
        if existing.is_none() {
            ensure_target(&transaction, &self.config, kind, id, now).await?;
        }
        let before = target::Entity::find_by_id((kind.to_owned(), id.to_owned()))
            .one(&transaction)
            .await?
            .ok_or_else(|| Error::internal("target row missing after insert"))?;
        let mut row = before.clone().into_active_model();
        let mut hidden = before.hidden;
        let mut state = before.state.clone();
        if let Some(value) = change.hidden {
            hidden = value;
            // Ẩn là đóng luôn.
            if value {
                state = "closed".to_owned();
            }
        }
        if let Some(value) = change.state {
            state = value;
        }
        row.state = Set(state);
        row.hidden = Set(hidden);
        if let Some(closes_at) = change.closes_at {
            row.closes_at = Set(closes_at);
        }
        row.updated_at = Set(now);
        let after = row.update(&transaction).await?;
        let (old, new) = (Target::from_row(before), Target::from_row(after));
        let snapshot = |target: &Target| {
            json!({
                "target_type": target.kind,
                "target_id": target.id,
                "state": target.effective_state(),
                "hidden": target.hidden,
                "closes_at": target.closes_at,
            })
        };
        let opened = new.is_open(now);
        if opened != old.is_open(now) || old.hidden != new.hidden {
            let change = Change { before: Some(snapshot(&old)), after: Some(snapshot(&new)) };
            let subject = events::target_subject(kind, id);
            if opened {
                self.outbox
                    .record(&transaction, actor, subject, &events::TargetOpened(change))
                    .await?;
            } else {
                self.outbox
                    .record(&transaction, actor, subject, &events::TargetClosed(change))
                    .await?;
            }
        }
        transaction.commit().await?;
        Ok(new)
    }
}

/// Event vòng đời `comments` nghe: những event module sở hữu đã khai cho loại đích đang bật.
pub(crate) fn owner_events(
    declared: &[ResourceEvent],
    target_types: &[String],
) -> Vec<ResourceEvent> {
    declared
        .iter()
        .filter(|event| target_types.iter().any(|known| known == event.kind))
        .copied()
        .collect()
}

/// Bên nhận event vòng đời của module sở hữu đích (`Module::resource_events`, D-202).
pub(crate) struct OwnerEvents {
    pub service: Arc<CommentsService>,
    pub events: Vec<ResourceEvent>,
}

#[async_trait::async_trait]
impl EventHandler for OwnerEvents {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let Some(declared) = self.events.iter().find(|declared| declared.event == event.name)
        else {
            return Ok(());
        };
        let service = &self.service;
        let kind = declared.kind;
        if !service.config.target_types.iter().any(|known| known == kind) {
            return Ok(());
        }
        let id = event
            .subject
            .as_deref()
            .and_then(|subject| declared.id_in(subject))
            .filter(|id| text::is_target_id(id));
        let Some(id) = id else {
            return Ok(());
        };
        // Kiểu `ResourceChange` có thể thêm biến thể về sau (`non_exhaustive`): biến thể lạ
        // tính như `Closed` — chỉ đóng, không ẩn.
        let hide = matches!(declared.change, ResourceChange::Removed);
        let change = TargetChange {
            state: Some("closed".to_owned()),
            hidden: hide.then_some(true),
            closes_at: None,
        };
        // Nhận lại cùng event: trạng thái đã đúng, không phát event mới.
        service.change_target(Actor::system(), kind, id, change, None).await?;
        tracing::info!(
            target_type = kind,
            target_id = id,
            hidden = hide,
            "comments closed by owner event"
        );
        Ok(())
    }
}
