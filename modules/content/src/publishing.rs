//! Đăng, gỡ, lưu trữ, xoá, chuyển trang (D-101, D-102). Mỗi việc một transaction, mở đầu
//! bằng khoá cây của ngôn ngữ (`paths::lock_tree`), và kiểm phiên bản (`If-Match`) ngay trong
//! câu `UPDATE` của mục: hai lệnh cùng phiên bản thì một bên nhận 412.

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use chrono::{DateTime, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, PaginatorTrait,
    QueryFilter, Set,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::body;
use crate::entity::{
    ROOT, entry, entry_author, entry_term, published, redirect, revision, schedule,
};
use crate::entries::{Entry, Kind, Status, changed_columns, is_unique_violation, summary};
use crate::events::{self, Change};
use crate::paths::{self, Generated, Moved};
use crate::permissions::{self, CONTENT_DELETE, CONTENT_PUBLISH};
use crate::revisions::{self, Reason};
use crate::service::{ContentService, entry_not_found, invalid, now};

/// Bản công khai của một mục.
pub(crate) type Published = published::Model;

/// Ai đăng, lúc nào, với ngày hiển thị nào, và ai xác nhận đã duyệt nội dung do AI tạo.
pub(crate) struct PublishStamp {
    /// Ngày hiển thị; `None`: giữ ngày cũ, hay `now` khi đăng lần đầu.
    pub published_at: Option<DateTime<Utc>>,
    pub now: DateTime<Utc>,
    pub by: Option<Uuid>,
    pub ai_reviewed_by: Option<Uuid>,
}

impl ContentService {
    /// Đăng bản nháp hiện tại của mục `id`. Cần `content.publish`, hoặc `content.publish_own`
    /// trên mục của mình. `published_at`: ngày hiển thị (mặc định: giữ ngày cũ, hay bây giờ nếu
    /// đăng lần đầu). `ai_reviewed`: người đăng xác nhận đã duyệt nội dung do AI tạo — chỉ một
    /// người (phiên đăng nhập) xác nhận được, API key thì bị từ chối.
    pub(crate) async fn publish(
        &self,
        principal: &Principal,
        id: Uuid,
        expected: Option<DateTime<Utc>>,
        published_at: Option<DateTime<Utc>>,
        ai_reviewed: bool,
    ) -> Result<Entry, Error> {
        permissions::require_some_publish(principal)?;
        if ai_reviewed && principal.api_key_id().is_some() {
            return Err(Error::forbidden(
                "content.ai_review_requires_human",
                "Only a person signed in (not an API key) can confirm that AI-generated content was reviewed.",
            ));
        }
        let entry = self.load_checked(id, expected).await?;
        permissions::require_publish(principal, &entry)?;
        // Ảnh phải tồn tại và công khai; hỏi `media` TRƯỚC transaction.
        self.check_publishable(&entry).await?;
        let now = now();
        let by = Some(principal.user_id());
        let transaction = self.database.begin_write().await?;
        paths::lock_tree(&transaction, &entry.locale).await?;
        let current = self.reload(&transaction, id, expected.unwrap_or(entry.updated_at)).await?;
        let revision =
            revisions::record(&transaction, &current, Reason::Published, by, now).await?;
        let reviewed_by = (ai_reviewed && current.ai_generated).then(|| principal.user_id());
        let (path, moved) = self
            .put_published(
                &transaction,
                &current,
                revision.id,
                PublishStamp { published_at, now, by, ai_reviewed_by: reviewed_by },
            )
            .await?;
        let mut after = current.clone();
        after.status = Status::Published.as_str().to_owned();
        after.has_unpublished_changes = false;
        // Đã đăng thì không còn chờ duyệt.
        after.review_requested_at = None;
        after.review_requested_by = None;
        after.updated_at = now;
        let after = self.save_state(&transaction, &current, &after).await?;
        let change = Change {
            before: Some(summary(&current)),
            after: Some(summary(&after)),
            published: Some(published_payload(&after, Some(&path), &moved)),
        };
        self.enqueue_index(&transaction, id).await?;
        self.outbox
            .record(
                &transaction,
                Actor::from(principal),
                events::entry_subject(id),
                &events::EntryPublished(change),
            )
            .await?;
        transaction.commit().await?;
        tracing::info!(entry_id = %id, path, "content entry published");
        Ok(after)
    }

    /// Gỡ mục `id`: không còn công khai (đường dẫn trả 404), trở lại nháp; đăng lại được.
    /// Cần `content.publish`.
    pub(crate) async fn unpublish(
        &self,
        principal: &Principal,
        id: Uuid,
        expected: Option<DateTime<Utc>>,
    ) -> Result<Entry, Error> {
        self.take_down(principal, id, expected, Status::Draft).await
    }

    /// Lưu trữ mục `id`: gỡ và ghi "đã gỡ" (410) ở đường dẫn của nó. Cần `content.publish`.
    pub(crate) async fn archive(
        &self,
        principal: &Principal,
        id: Uuid,
        expected: Option<DateTime<Utc>>,
    ) -> Result<Entry, Error> {
        self.take_down(principal, id, expected, Status::Archived).await
    }

    async fn take_down(
        &self,
        principal: &Principal,
        id: Uuid,
        expected: Option<DateTime<Utc>>,
        to: Status,
    ) -> Result<Entry, Error> {
        // Gỡ: `content.publish`, hay `content.publish_own` trên mục của mình. Lưu trữ (410, gỡ
        // hẳn) chỉ `content.publish`.
        if to == Status::Draft {
            permissions::require_some_publish(principal)?;
        } else {
            principal.require(&CONTENT_PUBLISH)?;
        }
        let entry = self.load_checked(id, expected).await?;
        if to == Status::Draft {
            permissions::require_publish(principal, &entry)?;
        }
        if to == Status::Draft && Status::of(&entry) != Status::Published {
            return Err(Error::conflict("content.not_published", "The entry is not published."));
        }
        let now = now();
        let by = Some(principal.user_id());
        let transaction = self.database.begin_write().await?;
        paths::lock_tree(&transaction, &entry.locale).await?;
        let current = self.reload(&transaction, id, expected.unwrap_or(entry.updated_at)).await?;
        let removed =
            self.remove_published(&transaction, &current, to == Status::Archived, by, now).await?;
        if to == Status::Draft {
            revisions::record(&transaction, &current, Reason::Unpublished, by, now).await?;
        }
        let mut after = current.clone();
        after.status = to.as_str().to_owned();
        after.has_unpublished_changes = false;
        after.updated_at = now;
        let after = self.save_state(&transaction, &current, &after).await?;
        let stale: Vec<Moved> =
            removed.iter().map(|path| (id, path.clone(), String::new())).collect();
        let change = Change {
            before: Some(summary(&current)),
            after: Some(summary(&after)),
            published: removed.as_ref().map(|_| published_payload(&after, None, &stale)),
        };
        self.enqueue_index(&transaction, id).await?;
        let actor = Actor::from(principal);
        let subject = events::entry_subject(id);
        if to == Status::Archived {
            self.outbox
                .record(&transaction, actor, subject, &events::EntryArchived(change))
                .await?;
        } else {
            self.outbox
                .record(&transaction, actor, subject, &events::EntryUnpublished(change))
                .await?;
        }
        transaction.commit().await?;
        Ok(after)
    }

    /// Xoá hẳn mục `id` (chỉ khi không còn trang con); đang công khai thì đường dẫn trả 410.
    /// Cần `content.delete` (tách khỏi `content.publish`, D-109).
    pub(crate) async fn delete(
        &self,
        principal: &Principal,
        id: Uuid,
        expected: Option<DateTime<Utc>>,
    ) -> Result<(), Error> {
        principal.require(&CONTENT_DELETE)?;
        let entry = self.load_checked(id, expected).await?;
        let now = now();
        let by = Some(principal.user_id());
        let transaction = self.database.begin_write().await?;
        paths::lock_tree(&transaction, &entry.locale).await?;
        let current = self.reload(&transaction, id, expected.unwrap_or(entry.updated_at)).await?;
        let children = entry::Entity::find()
            .filter(entry::Column::ParentKey.eq(id))
            .count(&transaction)
            .await?;
        if children > 0 {
            transaction.rollback().await?;
            return Err(Error::conflict(
                "content.page_has_children",
                "The page still has {count} subpages. Move or delete them first.",
            )
            .with_param("count", children));
        }
        let removed = self.remove_published(&transaction, &current, true, by, now).await?;
        entry::Entity::delete_by_id(id).exec(&transaction).await?;
        revision::Entity::delete_many()
            .filter(revision::Column::EntryId.eq(id))
            .exec(&transaction)
            .await?;
        schedule::Entity::delete_many()
            .filter(schedule::Column::EntryId.eq(id))
            .exec(&transaction)
            .await?;
        entry_term::Entity::delete_many()
            .filter(entry_term::Column::EntryId.eq(id))
            .exec(&transaction)
            .await?;
        entry_author::Entity::delete_many()
            .filter(entry_author::Column::EntryId.eq(id))
            .exec(&transaction)
            .await?;
        // Chuyển hướng trỏ tới mục này: đích không còn, thành "đã gỡ".
        redirect::Entity::update_many()
            .col_expr(redirect::Column::ToEntryId, Expr::value(Option::<Uuid>::None))
            .col_expr(redirect::Column::StatusCode, Expr::value(410))
            .col_expr(redirect::Column::Source, Expr::value("gone"))
            .col_expr(redirect::Column::UpdatedAt, Expr::value(now))
            .filter(redirect::Column::ToEntryId.eq(id))
            .exec(&transaction)
            .await?;
        let stale: Vec<Moved> =
            removed.iter().map(|path| (id, path.clone(), String::new())).collect();
        let change = Change {
            before: Some(summary(&current)),
            after: None,
            published: removed.as_ref().map(|_| published_payload(&current, None, &stale)),
        };
        self.enqueue_index(&transaction, id).await?;
        self.outbox
            .record(
                &transaction,
                Actor::from(principal),
                events::entry_subject(id),
                &events::EntryDeleted(change),
            )
            .await?;
        transaction.commit().await?;
        tracing::info!(entry_id = %id, "content entry deleted");
        Ok(())
    }

    /// Chuyển trang `id` sang cha `parent` (`None`: gốc). Có hiệu lực ngay cho cả nháp lẫn
    /// bản công khai: đường dẫn của nó và mọi trang con đổi, kèm chuyển hướng từ đường dẫn cũ.
    /// Cần `content.write` (hay `content.write_own` trên trang của mình), và `content.publish`
    /// nếu trang đang công khai.
    pub(crate) async fn move_page(
        &self,
        principal: &Principal,
        id: Uuid,
        parent: Option<Uuid>,
        expected: Option<DateTime<Utc>>,
    ) -> Result<Entry, Error> {
        permissions::require_create(principal)?;
        let entry = self.load_checked(id, expected).await?;
        permissions::require_write(principal, &entry)?;
        if entry.kind != Kind::Page.as_str() {
            return Err(invalid(vec![FieldError::new(
                "parent_id",
                "not_allowed",
                "only pages have a parent",
            )]));
        }
        if Status::of(&entry) == Status::Published {
            principal.require(&CONTENT_PUBLISH)?;
        }
        let now = now();
        let by = Some(principal.user_id());
        let transaction = self.database.begin_write().await?;
        paths::lock_tree(&transaction, &entry.locale).await?;
        let current = self.reload(&transaction, id, expected.unwrap_or(entry.updated_at)).await?;
        if let Some(parent) = parent {
            if crate::entries::depth(&transaction, parent).await.is_ok()
                && self.is_ancestor(&transaction, id, parent).await?
            {
                return Err(Error::conflict(
                    "content.tree_cycle",
                    "A page cannot move under itself or one of its subpages.",
                ));
            }
            let mut problems = Vec::new();
            self.check_parent(&transaction, &current, parent, &mut problems).await?;
            if !problems.is_empty() {
                return Err(invalid(problems));
            }
        }
        let mut after = current.clone();
        after.parent_id = parent;
        after.parent_key = parent.unwrap_or(ROOT);
        after.updated_at = now;
        let after = self.save_state(&transaction, &current, &after).await?;

        let mut moved = Vec::new();
        let mut path = None;
        if let Some(old) = published::Entity::find_by_id(id).one(&transaction).await? {
            // Đường dẫn công khai mới tính từ slug ĐÃ ĐĂNG: slug của nháp chỉ ra ngoài khi đăng.
            let mut moved_entry = after.clone();
            moved_entry.slug.clone_from(&old.slug);
            let new = paths::path_of(&transaction, &moved_entry, &self.config.post_path_prefix)
                .await?
                .map_err(|()| parent_not_published())?;
            if paths::path_used(&transaction, &after.locale, &new, id).await? {
                return Err(path_taken(&new));
            }
            let mut row = old.clone().into_active_model();
            row.parent_id = Set(parent);
            row.path = Set(new.clone());
            row.updated_at = Set(now);
            row.update(&transaction).await.map_err(|error| unique_path(error, &new))?;
            if old.path != new {
                moved.push((id, old.path.clone(), new.clone()));
                moved.extend(
                    paths::repath_descendants(&transaction, id, &old.path, &new, now).await?,
                );
                paths::redirect_moved(&transaction, &after.locale, &moved, by, now).await?;
            }
            path = Some(new);
        }
        let change = Change {
            before: Some(summary(&current)),
            after: Some(summary(&after)),
            published: path.as_ref().map(|path| published_payload(&after, Some(path), &moved)),
        };
        self.outbox
            .record(
                &transaction,
                Actor::from(principal),
                events::entry_subject(id),
                &events::EntryMoved(change),
            )
            .await?;
        transaction.commit().await?;
        Ok(after)
    }

    /// Chép phiên bản `version` (thường là bản nháp) của mục sang bản công khai, trong
    /// `transaction` (đã khoá cây). Trả đường dẫn mới và các đường dẫn đã đổi.
    pub(crate) async fn put_published<C: ConnectionTrait>(
        &self,
        transaction: &C,
        version: &Entry,
        revision_id: Uuid,
        stamp: PublishStamp,
    ) -> Result<(String, Vec<Moved>), Error> {
        let PublishStamp { published_at, now, by, ai_reviewed_by } = stamp;
        let path = paths::path_of(transaction, version, &self.config.post_path_prefix)
            .await?
            .map_err(|()| parent_not_published())?;
        if paths::path_used(transaction, &version.locale, &path, version.id).await? {
            return Err(path_taken(&path));
        }
        let old = published::Entity::find_by_id(version.id).one(transaction).await?;
        let row = published::Model {
            entry_id: version.id,
            kind: version.kind.clone(),
            locale: version.locale.clone(),
            translation_group_id: version.translation_group_id,
            parent_id: version.parent_id,
            revision_id,
            path: path.clone(),
            title: version.title.clone(),
            slug: version.slug.clone(),
            excerpt: version.excerpt.clone(),
            body: version.body.clone(),
            body_schema_version: version.body_schema_version,
            plain_text: version.plain_text.clone(),
            search_key: version.search_key.clone(),
            word_count: version.word_count,
            reading_time_minutes: version.reading_time_minutes,
            featured_media_id: version.featured_media_id,
            featured_media_alt: version.featured_media_alt.clone(),
            meta_title: version.meta_title.clone(),
            meta_description: version.meta_description.clone(),
            og_media_id: version.og_media_id,
            canonical_url: version.canonical_url.clone(),
            noindex: version.noindex,
            ai_generated: version.ai_generated,
            ai_reviewed_by,
            ai_reviewed_at: ai_reviewed_by.map(|_| now),
            published_at: published_at.or(old.as_ref().map(|old| old.published_at)).unwrap_or(now),
            first_published_at: old.as_ref().map_or(now, |old| old.first_published_at),
            content_modified_at: now,
            updated_at: now,
        };
        let mut moved = Vec::new();
        match &old {
            Some(old) => {
                // Mọi cột đổi theo phiên bản mới; khoá chính giữ nguyên.
                let mut active = row.into_active_model().reset_all();
                active.entry_id = sea_orm::ActiveValue::Unchanged(version.id);
                active.update(transaction).await.map_err(|error| unique_path(error, &path))?;
                if old.path != path {
                    moved.push((version.id, old.path.clone(), path.clone()));
                    moved.extend(
                        paths::repath_descendants(transaction, version.id, &old.path, &path, now)
                            .await?,
                    );
                }
            }
            None => {
                row.into_active_model()
                    .insert(transaction)
                    .await
                    .map_err(|error| unique_path(error, &path))?;
            }
        }
        paths::clear_automatic(transaction, &version.locale, &path).await?;
        paths::redirect_moved(transaction, &version.locale, &moved, by, now).await?;
        Ok((path, moved))
    }

    /// Bỏ bản công khai của `entry` (nếu có); `gone`: ghi 410 ở đường dẫn của nó. Không cho
    /// gỡ trang còn trang con đang công khai. Trả đường dẫn đã bỏ.
    pub(crate) async fn remove_published<C: ConnectionTrait>(
        &self,
        transaction: &C,
        entry: &Entry,
        gone: bool,
        by: Option<Uuid>,
        now: DateTime<Utc>,
    ) -> Result<Option<String>, Error> {
        let Some(old) = published::Entity::find_by_id(entry.id).one(transaction).await? else {
            return Ok(None);
        };
        let children = published::Entity::find()
            .filter(published::Column::ParentId.eq(entry.id))
            .count(transaction)
            .await?;
        if children > 0 {
            return Err(Error::conflict(
                "content.page_has_published_children",
                "The page has {count} published subpages. Take them down or move them first.",
            )
            .with_param("count", children));
        }
        published::Entity::delete_by_id(entry.id).exec(transaction).await?;
        if gone {
            let row = Generated {
                locale: &entry.locale,
                from: &old.path,
                to_entry: None,
                status: 410,
                source: "gone",
                by,
                at: now,
            };
            paths::put_redirect(transaction, row).await?;
        }
        Ok(Some(old.path))
    }

    /// Ảnh của mục phải tồn tại và công khai; thân bài phải hợp lệ.
    pub(crate) async fn check_publishable(&self, entry: &Entry) -> Result<(), Error> {
        let body: Value = serde_json::from_str(&entry.body).unwrap_or_else(|_| body::empty());
        let mut problems = Vec::new();
        let info = match body::check(&body, self.config.max_body_bytes, &self.config.embed_hosts) {
            Ok(info) => info,
            Err(errors) => {
                problems.extend(errors);
                body::BodyInfo::default()
            }
        };
        self.check_media(entry, &info.media_ids, true, &mut problems).await?;
        if problems.is_empty() { Ok(()) } else { Err(invalid(problems)) }
    }

    /// Đọc mục `id`, kiểm phiên bản người gọi đã đọc (412 nếu khác).
    pub(crate) async fn load_checked(
        &self,
        id: Uuid,
        expected: Option<DateTime<Utc>>,
    ) -> Result<Entry, Error> {
        let entry = entry::Entity::find_by_id(id)
            .one(self.database.read())
            .await?
            .ok_or_else(|| entry_not_found(id))?;
        if expected.is_some_and(|expected| expected != entry.updated_at) {
            return Err(autoxide_kernel::version::changed());
        }
        Ok(entry)
    }

    /// Đọc lại mục trong `transaction`; đã đổi khác `version` thì 412.
    pub(crate) async fn reload<C: ConnectionTrait>(
        &self,
        transaction: &C,
        id: Uuid,
        version: DateTime<Utc>,
    ) -> Result<Entry, Error> {
        let entry = entry::Entity::find_by_id(id)
            .one(transaction)
            .await?
            .ok_or_else(|| entry_not_found(id))?;
        if entry.updated_at != version {
            return Err(autoxide_kernel::version::changed());
        }
        Ok(entry)
    }

    /// Ghi các cột đổi từ `before` sang `after`, với điều kiện phiên bản vẫn là của `before`.
    pub(crate) async fn save_state<C: ConnectionTrait>(
        &self,
        transaction: &C,
        before: &Entry,
        after: &Entry,
    ) -> Result<Entry, Error> {
        let update = entry::Entity::update(changed_columns(before, after))
            .validate()?
            .filter(entry::Column::UpdatedAt.eq(before.updated_at));
        match update.exec(transaction).await {
            Ok(saved) => Ok(saved),
            Err(sea_orm::DbErr::RecordNotUpdated) => Err(autoxide_kernel::version::changed()),
            Err(error) if is_unique_violation(&error) => Err(Error::conflict(
                "content.slug_taken",
                "The slug `{slug}` is already used by another entry in the same language and place.",
            )
            .with_param("slug", &after.slug)),
            Err(error) => Err(error.into()),
        }
    }

    /// Trang `ancestor` có phải `id` hay tổ tiên của `id` không (đi ngược lên từ `id`).
    async fn is_ancestor<C: ConnectionTrait>(
        &self,
        connection: &C,
        ancestor: Uuid,
        id: Uuid,
    ) -> Result<bool, Error> {
        let mut current = Some(id);
        for _ in 0..100 {
            let Some(id) = current else { return Ok(false) };
            if id == ancestor {
                return Ok(true);
            }
            current =
                entry::Entity::find_by_id(id).one(connection).await?.and_then(|row| row.parent_id);
        }
        Err(Error::internal("the page tree is deeper than expected; it may contain a cycle"))
    }
}

/// Dữ liệu cho website làm mới cache: mục, đường dẫn mới, mọi đường dẫn cũ bị ảnh hưởng.
pub(crate) fn published_payload(entry: &Entry, path: Option<&str>, moved: &[Moved]) -> Value {
    let stale: Vec<&str> = moved.iter().map(|(_, old, _)| old.as_str()).collect();
    let changed: Vec<Value> = moved
        .iter()
        .filter(|(id, _, new)| *id != entry.id && !new.is_empty())
        .map(|(id, old, new)| json!({"entry_id": id, "old_path": old, "path": new}))
        .collect();
    json!({
        "entry_id": entry.id,
        "kind": entry.kind,
        "locale": entry.locale,
        "translation_group_id": entry.translation_group_id,
        "path": path,
        "stale_paths": stale,
        "moved_subpages": changed,
    })
}

fn parent_not_published() -> Error {
    Error::conflict(
        "content.parent_not_published",
        "The parent page is not published yet. Publish it first.",
    )
}

pub(crate) fn path_taken(path: &str) -> Error {
    Error::conflict(
        "content.path_taken",
        "The address `{path}` is already used by another published entry in this language.",
    )
    .with_param("path", path)
}

/// Vi phạm unique (ngôn ngữ, đường dẫn) khi ghi bản công khai → `content.path_taken`.
fn unique_path(error: sea_orm::DbErr, path: &str) -> Error {
    if is_unique_violation(&error) { path_taken(path) } else { error.into() }
}
