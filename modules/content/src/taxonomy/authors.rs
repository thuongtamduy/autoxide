//! Tác giả: hồ sơ của `content` (D-102) — tên hiển thị, slug, giới thiệu, ảnh — giữ
//! `user_id` của tài khoản (không foreign key, không phụ thuộc module `auth`). Một mục có
//! nhiều tác giả theo thứ tự (người đầu là chính); gắn tác giả có hiệu lực ngay.

use std::collections::HashMap;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::TenantContext;
use chrono::{DateTime, Utc};
use sea_orm::sea_query::Query;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter,
    QueryOrder, Set,
};
use serde_json::json;
use uuid::Uuid;

use crate::entity::{author, entry, entry_author, published};
use crate::events::{self, Change};
use crate::permissions::{CONTENT_MANAGE, CONTENT_READ};
use crate::service::{ContentService, check_length, check_url, invalid, now};

pub(crate) type Author = author::Model;

const MAX_PER_ENTRY: usize = 20;

/// Một tác giả như hiển thị cùng mục.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AuthorRef {
    pub id: Uuid,
    pub display_name: String,
    pub slug: String,
    pub avatar_media_id: Option<Uuid>,
}

/// Thay đổi hồ sơ tác giả; `None` giữ nguyên.
#[derive(Debug, Clone, Default)]
pub(crate) struct AuthorInput {
    /// `Some(None)`: bỏ liên kết tài khoản.
    pub user_id: Option<Option<Uuid>>,
    pub display_name: Option<String>,
    pub slug: Option<String>,
    pub bio: Option<String>,
    pub avatar_media_id: Option<Option<Uuid>>,
    pub website_url: Option<String>,
}

/// Điều kiện "mục có tác giả `author_id`", cho bộ lọc danh sách quản trị.
pub(crate) fn has_author(author_id: Uuid) -> sea_orm::sea_query::SimpleExpr {
    entry::Column::Id.in_subquery(by_author(author_id))
}

/// Như [`has_author`], cho bảng bản công khai.
pub(crate) fn published_has_author(author_id: Uuid) -> sea_orm::sea_query::SimpleExpr {
    published::Column::EntryId.in_subquery(by_author(author_id))
}

fn by_author(author_id: Uuid) -> sea_orm::sea_query::SelectStatement {
    Query::select()
        .column(entry_author::Column::EntryId)
        .from(entry_author::Entity)
        .and_where(entry_author::Column::AuthorId.eq(author_id))
        .to_owned()
}

/// ID tác giả của từng mục trong `ids`, theo thứ tự.
pub(crate) async fn author_ids_of<C: ConnectionTrait>(
    connection: &C,
    ids: &[Uuid],
) -> Result<HashMap<Uuid, Vec<Uuid>>, Error> {
    let mut found: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    if ids.is_empty() {
        return Ok(found);
    }
    for link in entry_author::Entity::find()
        .filter(entry_author::Column::EntryId.is_in(ids.to_vec()))
        .order_by_asc(entry_author::Column::SortOrder)
        .all(connection)
        .await?
    {
        found.entry(link.entry_id).or_default().push(link.author_id);
    }
    Ok(found)
}

/// Tác giả của từng mục trong `ids` (theo thứ tự; người đầu là chính).
pub(crate) async fn authors_of<C: ConnectionTrait>(
    connection: &C,
    ids: &[Uuid],
) -> Result<HashMap<Uuid, Vec<AuthorRef>>, Error> {
    let links = author_ids_of(connection, ids).await?;
    let author_ids: Vec<Uuid> = links.values().flatten().copied().collect();
    if author_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let authors: HashMap<Uuid, Author> = author::Entity::find()
        .filter(author::Column::Id.is_in(author_ids))
        .all(connection)
        .await?
        .into_iter()
        .map(|row| (row.id, row))
        .collect();
    Ok(links
        .into_iter()
        .map(|(entry, ids)| {
            let refs = ids
                .iter()
                .filter_map(|id| authors.get(id))
                .map(|row| AuthorRef {
                    id: row.id,
                    display_name: row.display_name.clone(),
                    slug: row.slug.clone(),
                    avatar_media_id: row.avatar_media_id,
                })
                .collect();
            (entry, refs)
        })
        .collect())
}

impl ContentService {
    /// ID tác giả của từng mục trong `ids`, theo thứ tự (để API trả mục kèm tác giả).
    pub(crate) async fn author_ids(&self, ids: &[Uuid]) -> Result<HashMap<Uuid, Vec<Uuid>>, Error> {
        author_ids_of(self.database.read(), ids).await
    }

    pub(crate) async fn list_authors(
        &self,
        principal: &Principal,
        page: &PageRequest,
    ) -> Result<Page<Author>, Error> {
        principal.require(&CONTENT_READ)?;
        fetch_page(
            author::Entity::find(),
            author::Column::Id,
            |row| row.id,
            page,
            self.database.read(),
        )
        .await
    }

    pub(crate) async fn get_author(
        &self,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Author, Error> {
        principal.require(&CONTENT_READ)?;
        author::Entity::find_by_id(id)
            .one(self.database.read())
            .await?
            .ok_or_else(|| author_not_found(id))
    }

    /// Tạo hồ sơ tác giả. Cần `content.manage`.
    pub(crate) async fn create_author(
        &self,
        principal: &Principal,
        input: AuthorInput,
    ) -> Result<Author, Error> {
        principal.require(&CONTENT_MANAGE)?;
        let now = now();
        let id = Uuid::now_v7();
        let mut row = Author {
            id,
            user_id: None,
            display_name: String::new(),
            slug: String::new(),
            bio: String::new(),
            avatar_media_id: None,
            website_url: String::new(),
            created_at: now,
            updated_at: now,
        };
        self.apply_author(&mut row, input).await?;
        let transaction = self.database.begin_write().await?;
        let saved =
            row.into_active_model().insert(&transaction).await.map_err(author_slug_conflict)?;
        let change = Change { before: None, after: Some(author_summary(&saved)), published: None };
        self.outbox
            .record(
                &transaction,
                &TenantContext::legacy_default(&self.database),
                Actor::from(principal),
                events::subject("author", id),
                &events::AuthorChanged(change),
            )
            .await?;
        transaction.commit().await?;
        Ok(saved)
    }

    /// Sửa hồ sơ tác giả. Cần `content.manage`.
    pub(crate) async fn update_author(
        &self,
        principal: &Principal,
        id: Uuid,
        input: AuthorInput,
        expected: Option<DateTime<Utc>>,
    ) -> Result<Author, Error> {
        principal.require(&CONTENT_MANAGE)?;
        let before = author::Entity::find_by_id(id)
            .one(self.database.read())
            .await?
            .ok_or_else(|| author_not_found(id))?;
        let mut row = before.clone();
        self.apply_author(&mut row, input).await?;
        row.updated_at = now();
        let transaction = self.database.begin_write().await?;
        let mut active = row.into_active_model().reset_all();
        active.id = sea_orm::ActiveValue::Unchanged(id);
        let after = author::Entity::update(active)
            .validate()?
            .filter(author::Column::UpdatedAt.eq(expected.unwrap_or(before.updated_at)))
            .exec(&transaction)
            .await
            .map_err(|error| match error {
                sea_orm::DbErr::RecordNotUpdated => autoxide_kernel::version::changed(),
                error => author_slug_conflict(error),
            })?;
        let change = Change {
            before: Some(author_summary(&before)),
            after: Some(author_summary(&after)),
            published: None,
        };
        self.outbox
            .record(
                &transaction,
                &TenantContext::legacy_default(&self.database),
                Actor::from(principal),
                events::subject("author", id),
                &events::AuthorChanged(change),
            )
            .await?;
        transaction.commit().await?;
        Ok(after)
    }

    /// Xoá hồ sơ tác giả (gỡ khỏi mọi mục). Cần `content.manage`.
    pub(crate) async fn delete_author(&self, principal: &Principal, id: Uuid) -> Result<(), Error> {
        principal.require(&CONTENT_MANAGE)?;
        let actor = Actor::from(principal);
        self.remove_author(id, actor).await?.ok_or_else(|| author_not_found(id))?;
        Ok(())
    }

    /// Xoá tác giả `id` và liên kết với mục; `None` nếu không có.
    pub(crate) async fn remove_author(
        &self,
        id: Uuid,
        actor: Actor,
    ) -> Result<Option<Author>, Error> {
        let transaction = self.database.begin_write().await?;
        let Some(before) = author::Entity::find_by_id(id).one(&transaction).await? else {
            return Ok(None);
        };
        author::Entity::delete_by_id(id).exec(&transaction).await?;
        entry_author::Entity::delete_many()
            .filter(entry_author::Column::AuthorId.eq(id))
            .exec(&transaction)
            .await?;
        let change = Change { before: Some(author_summary(&before)), after: None, published: None };
        self.outbox
            .record(
                &transaction,
                &TenantContext::legacy_default(&self.database),
                actor,
                events::subject("author", id),
                &events::AuthorChanged(change),
            )
            .await?;
        transaction.commit().await?;
        Ok(Some(before))
    }

    /// Thay danh sách tác giả của mục `entry_id` (theo thứ tự). Có hiệu lực ngay. Cần
    /// `content.write` (hay `content.write_own` trên mục của mình; không cần `content.read`, như
    /// mọi thao tác sửa mục — D-111), và quyền đăng khi mục đang công khai. Người chỉ có
    /// `content.write_own` chỉ thêm được hồ sơ tác giả gắn với chính tài khoản của mình (hồ sơ
    /// đã có trên mục thì giữ hay bỏ được): không đứng tên người khác.
    pub(crate) async fn set_entry_authors(
        &self,
        principal: &Principal,
        entry_id: Uuid,
        author_ids: Vec<Uuid>,
    ) -> Result<Vec<Uuid>, Error> {
        crate::permissions::require_create(principal)?;
        let entry = self.load_checked(entry_id, None).await?;
        crate::permissions::require_write(principal, &entry)?;
        let mut ids: Vec<Uuid> = Vec::new();
        for id in author_ids {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        if ids.len() > MAX_PER_ENTRY {
            return Err(invalid(vec![FieldError::new(
                "author_ids",
                "length",
                format!("at most {MAX_PER_ENTRY} authors"),
            )]));
        }
        let transaction = self.database.begin_write().await?;
        // Khoá dòng mục trước khi xoá rồi chèn lại liên kết (xem `set_entry_terms`).
        let entry = crate::entries::lock_entry(&transaction, entry.id).await?;
        if crate::entries::Status::of(&entry) == crate::entries::Status::Published {
            // Gắn có hiệu lực ngay cả trên bản công khai: như mọi thay đổi người đọc thấy, mục
            // đang công khai thì cần quyền đăng.
            crate::permissions::require_publish(principal, &entry)?;
        }
        let profiles = author::Entity::find()
            .filter(author::Column::Id.is_in(ids.clone()))
            .all(&transaction)
            .await?;
        if profiles.len() != ids.len() {
            return Err(invalid(vec![FieldError::new(
                "author_ids",
                "not_found",
                "every id must be an existing author",
            )]));
        }
        let before =
            author_ids_of(&transaction, &[entry_id]).await?.remove(&entry_id).unwrap_or_default();
        if crate::permissions::write_own_only(principal)
            && profiles.iter().any(|profile| {
                profile.user_id != Some(principal.user_id()) && !before.contains(&profile.id)
            })
        {
            return Err(invalid(vec![FieldError::new(
                "author_ids",
                "not_allowed",
                "with `content.write_own` you may only add author profiles linked to your own account",
            )]));
        }
        entry_author::Entity::delete_many()
            .filter(entry_author::Column::EntryId.eq(entry_id))
            .exec(&transaction)
            .await?;
        for (order, author_id) in ids.iter().enumerate() {
            entry_author::ActiveModel {
                entry_id: Set(entry_id),
                author_id: Set(*author_id),
                sort_order: Set(i32::try_from(order).unwrap_or(i32::MAX)),
            }
            .insert(&transaction)
            .await?;
        }
        self.record_link_change(&transaction, principal, &entry, "author_ids", before, ids.clone())
            .await?;
        transaction.commit().await?;
        Ok(ids)
    }

    /// Tác giả công khai theo slug.
    pub(crate) async fn author_by_slug(&self, slug: &str) -> Result<Author, Error> {
        author::Entity::find()
            .filter(author::Column::Slug.eq(slug))
            .one(self.database.read())
            .await?
            .ok_or_else(|| {
                Error::not_found("content.author_not_found", "No author has the slug `{slug}`.")
                    .with_param("slug", slug)
            })
    }

    async fn apply_author(&self, row: &mut Author, input: AuthorInput) -> Result<(), Error> {
        let mut problems = Vec::new();
        check_length("display_name", input.display_name.as_deref(), 200, &mut problems);
        check_length("bio", input.bio.as_deref(), 5000, &mut problems);
        check_url("website_url", input.website_url.as_deref(), &mut problems);
        if let Some(name) = input.display_name {
            row.display_name = name.trim().to_owned();
        }
        if row.display_name.is_empty() && !problems.iter().any(|p| p.field == "display_name") {
            problems.push(FieldError::new(
                "display_name",
                "required",
                "must be 1 to 200 characters",
            ));
        }
        if let Some(bio) = input.bio {
            row.bio = bio.trim().to_owned();
        }
        if let Some(url) = input.website_url {
            row.website_url = url.trim().to_owned();
        }
        if let Some(user) = input.user_id {
            row.user_id = user;
        }
        if let Some(avatar) = input.avatar_media_id {
            row.avatar_media_id = avatar;
            if let Some(id) = avatar
                && self
                    .media
                    .find(&TenantContext::legacy_default(&self.database), id)
                    .await?
                    .is_none()
            {
                problems.push(FieldError::new(
                    "avatar_media_id",
                    "media_not_found",
                    format!("media item {id} does not exist"),
                ));
            }
        }
        match input.slug.as_deref().map(str::trim) {
            Some(slug) if !slug.is_empty() => {
                if !crate::entries::slug::is_valid(slug) {
                    problems.push(FieldError::new(
                        "slug",
                        "invalid",
                        "must be lower-case letters and digits joined by `-`",
                    ));
                }
                row.slug = slug.to_owned();
            }
            _ if row.slug.is_empty() => {
                row.slug = crate::entries::slug::from_title(&row.display_name, row.id)
            }
            _ => {}
        }
        if problems.is_empty() { Ok(()) } else { Err(invalid(problems)) }
    }
}

fn author_summary(row: &Author) -> serde_json::Value {
    json!({
        "user_id": row.user_id,
        "display_name": row.display_name,
        "slug": row.slug,
        "avatar_media_id": row.avatar_media_id,
        "website_url": row.website_url,
    })
}

fn author_slug_conflict(error: sea_orm::DbErr) -> Error {
    if crate::entries::is_unique_violation(&error) {
        Error::conflict("content.slug_taken", "Another author already uses this slug.")
    } else {
        error.into()
    }
}

pub(crate) fn author_not_found(id: Uuid) -> Error {
    Error::not_found("content.author_not_found", "No author has the id `{id}`.")
        .with_param("id", id)
}
