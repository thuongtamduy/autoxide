//! Tác giả: hồ sơ của `content` (D-102) — tên hiển thị, slug, giới thiệu, ảnh — giữ
//! `user_id` của tài khoản (không foreign key, không phụ thuộc module `auth`). Một mục có
//! nhiều tác giả theo thứ tự (người đầu là chính); gắn tác giả có hiệu lực ngay.

use autoxide_kernel::time::Timestamp;
use std::collections::HashMap;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::{Actor, Outbox};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::sea_query::Query;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter,
    QueryOrder, QuerySelect, Set,
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
pub(crate) fn has_author(
    tenant: &TenantContext,
    author_id: Uuid,
) -> sea_orm::sea_query::SimpleExpr {
    entry::Column::Id.in_subquery(by_author(tenant, author_id))
}

/// Như [`has_author`], cho bảng bản công khai.
pub(crate) fn published_has_author(
    tenant: &TenantContext,
    author_id: Uuid,
) -> sea_orm::sea_query::SimpleExpr {
    published::Column::EntryId.in_subquery(by_author(tenant, author_id))
}

fn by_author(tenant: &TenantContext, author_id: Uuid) -> sea_orm::sea_query::SelectStatement {
    Query::select()
        .column(entry_author::Column::EntryId)
        .from(entry_author::Entity)
        .and_where(entry_author::Column::TenantId.eq(tenant.id().as_uuid()))
        .and_where(entry_author::Column::AuthorId.eq(author_id))
        .to_owned()
}

/// ID tác giả của từng mục trong `ids`, theo thứ tự.
pub(crate) async fn author_ids_of<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    ids: &[Uuid],
) -> Result<HashMap<Uuid, Vec<Uuid>>, Error> {
    let mut found: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    if ids.is_empty() {
        return Ok(found);
    }
    for link in entry_author::Entity::find()
        .scoped(tenant)
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
    tenant: &TenantContext,
    ids: &[Uuid],
) -> Result<HashMap<Uuid, Vec<AuthorRef>>, Error> {
    let links = author_ids_of(connection, tenant, ids).await?;
    let author_ids: Vec<Uuid> = links.values().flatten().copied().collect();
    if author_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let authors: HashMap<Uuid, Author> = author::Entity::find()
        .scoped(tenant)
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
    pub(crate) async fn author_ids(
        &self,
        tenant: &TenantContext,
        ids: &[Uuid],
    ) -> Result<HashMap<Uuid, Vec<Uuid>>, Error> {
        author_ids_of(tenant.database().read(), tenant, ids).await
    }

    pub(crate) async fn list_authors(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        search: Option<&str>,
        page: &PageRequest,
    ) -> Result<Page<Author>, Error> {
        principal.require(&CONTENT_READ)?;
        let mut select = author::Entity::find().scoped(tenant);
        if let Some(search) = search.map(str::trim).filter(|search| !search.is_empty()) {
            // Như tìm danh mục: so tên bỏ dấu trong bộ nhớ.
            let wanted = autoxide_kernel::search::fold(search);
            let names: Vec<(Uuid, String)> = author::Entity::find()
                .scoped(tenant)
                .select_only()
                .column(author::Column::Id)
                .column(author::Column::DisplayName)
                .into_tuple()
                .all(tenant.database().read())
                .await?;
            let ids: Vec<Uuid> = names
                .into_iter()
                .filter(|(_, name)| autoxide_kernel::search::fold(name).contains(&wanted))
                .map(|(id, _)| id)
                .collect();
            select = select.filter(author::Column::Id.is_in(ids));
        }
        fetch_page(select, author::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    /// Số bài (mọi trạng thái) của từng tác giả trong `ids` — một câu cho cả trang.
    pub(crate) async fn author_entry_counts(
        &self,
        tenant: &TenantContext,
        ids: &[Uuid],
    ) -> Result<HashMap<Uuid, u64>, Error> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let rows: Vec<(Uuid, i64)> = entry_author::Entity::find()
            .scoped(tenant)
            .select_only()
            .column(entry_author::Column::AuthorId)
            .column_as(entry_author::Column::EntryId.count(), "entries")
            .filter(entry_author::Column::AuthorId.is_in(ids.iter().copied()))
            .group_by(entry_author::Column::AuthorId)
            .into_tuple()
            .all(tenant.database().read())
            .await?;
        Ok(rows.into_iter().map(|(id, count)| (id, u64::try_from(count).unwrap_or(0))).collect())
    }

    pub(crate) async fn get_author(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Author, Error> {
        principal.require(&CONTENT_READ)?;
        author::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| author_not_found(id))
    }

    /// Tạo hồ sơ tác giả. Cần `content.manage`.
    pub(crate) async fn create_author(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: AuthorInput,
    ) -> Result<Author, Error> {
        principal.require(&CONTENT_MANAGE)?;
        let now = now();
        let id = Uuid::now_v7();
        let mut row = Author {
            id,
            tenant_id: tenant.id().as_uuid(),
            user_id: None,
            display_name: String::new(),
            slug: String::new(),
            bio: String::new(),
            avatar_media_id: None,
            website_url: String::new(),
            created_at: now,
            updated_at: now,
        };
        self.apply_author(tenant, &mut row, input).await?;
        let transaction = tenant.database().begin_write().await?;
        let saved =
            row.into_active_model().insert(&transaction).await.map_err(author_slug_conflict)?;
        let change = Change { before: None, after: Some(author_summary(&saved)), published: None };
        self.outbox
            .record(
                &transaction,
                tenant,
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
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        input: AuthorInput,
        expected: Option<Timestamp>,
    ) -> Result<Author, Error> {
        principal.require(&CONTENT_MANAGE)?;
        let before = author::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| author_not_found(id))?;
        let mut row = before.clone();
        self.apply_author(tenant, &mut row, input).await?;
        row.updated_at = now();
        let transaction = tenant.database().begin_write().await?;
        let mut active = row.into_active_model().reset_all();
        active.id = sea_orm::ActiveValue::Unchanged(id);
        let after = author::Entity::update(active)
            .validate()?
            .scoped(tenant)
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
                tenant,
                Actor::from(principal),
                events::subject("author", id),
                &events::AuthorChanged(change),
            )
            .await?;
        transaction.commit().await?;
        Ok(after)
    }

    /// Xoá hồ sơ tác giả (gỡ khỏi mọi mục). Cần `content.manage`.
    pub(crate) async fn delete_author(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        principal.require(&CONTENT_MANAGE)?;
        let actor = Actor::from(principal);
        self.remove_author(tenant, id, actor).await?.ok_or_else(|| author_not_found(id))?;
        Ok(())
    }

    /// Xoá tác giả `id` và liên kết với mục; `None` nếu không có.
    pub(crate) async fn remove_author(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        actor: Actor,
    ) -> Result<Option<Author>, Error> {
        remove_author_with(tenant, Some(&self.outbox), id, actor).await
    }

    /// Thay danh sách tác giả của mục `entry_id` (theo thứ tự). Có hiệu lực ngay. Cần
    /// `content.write` (hay `content.write_own` trên mục của mình; không cần `content.read`, như
    /// mọi thao tác sửa mục — D-111), và quyền đăng khi mục đang công khai. Người chỉ có
    /// `content.write_own` chỉ thêm được hồ sơ tác giả gắn với chính tài khoản của mình (hồ sơ
    /// đã có trên mục thì giữ hay bỏ được): không đứng tên người khác.
    pub(crate) async fn set_entry_authors(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        entry_id: Uuid,
        author_ids: Vec<Uuid>,
    ) -> Result<Vec<Uuid>, Error> {
        crate::permissions::require_create(principal)?;
        let entry = self.load_checked(tenant, entry_id, None).await?;
        crate::permissions::require_write(principal, &entry)?;
        crate::permissions::require_org(principal, tenant, &entry, crate::permissions::Need::Write)
            .await?;
        let mut ids: Vec<Uuid> = Vec::new();
        for id in author_ids {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        if ids.len() > MAX_PER_ENTRY {
            return Err(invalid(vec![
                FieldError::new("author_ids", "length", "at most {max_per_entry} authors")
                    .with_param("max_per_entry", MAX_PER_ENTRY),
            ]));
        }
        let transaction = tenant.database().begin_write().await?;
        // Khoá dòng mục trước khi xoá rồi chèn lại liên kết (xem `set_entry_terms`).
        let entry = crate::entries::lock_entry(&transaction, tenant, entry.id).await?;
        if crate::entries::Status::of(&entry) == crate::entries::Status::Published {
            // Gắn có hiệu lực ngay cả trên bản công khai: như mọi thay đổi người đọc thấy, mục
            // đang công khai thì cần quyền đăng.
            crate::permissions::require_publish(principal, &entry)?;
            crate::permissions::require_org(
                principal,
                tenant,
                &entry,
                crate::permissions::Need::Publish,
            )
            .await?;
        }
        let profiles = author::Entity::find()
            .scoped(tenant)
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
        let before = author_ids_of(&transaction, tenant, &[entry_id])
            .await?
            .remove(&entry_id)
            .unwrap_or_default();
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
            .scoped(tenant)
            .filter(entry_author::Column::EntryId.eq(entry_id))
            .exec(&transaction)
            .await?;
        for (order, author_id) in ids.iter().enumerate() {
            entry_author::ActiveModel {
                tenant_id: Set(tenant.id().as_uuid()),
                entry_id: Set(entry_id),
                author_id: Set(*author_id),
                sort_order: Set(i32::try_from(order).unwrap_or(i32::MAX)),
            }
            .insert(&transaction)
            .await?;
        }
        self.record_link_change(
            tenant,
            &transaction,
            principal,
            &entry,
            crate::taxonomy::terms::LinkChange { field: "author_ids", before, after: ids.clone() },
        )
        .await?;
        transaction.commit().await?;
        Ok(ids)
    }

    /// Tác giả công khai theo slug.
    pub(crate) async fn author_by_slug(
        &self,
        tenant: &TenantContext,
        slug: &str,
    ) -> Result<Author, Error> {
        author::Entity::find()
            .scoped(tenant)
            .filter(author::Column::Slug.eq(slug))
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| {
                Error::not_found("content.author_not_found", "No author has the slug `{slug}`.")
                    .with_param("slug", slug)
            })
    }

    async fn apply_author(
        &self,
        tenant: &TenantContext,
        row: &mut Author,
        input: AuthorInput,
    ) -> Result<(), Error> {
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
                && self.media.find(tenant, id).await?.is_none()
            {
                problems.push(
                    FieldError::new(
                        "avatar_media_id",
                        "media_not_found",
                        "media item {id} does not exist",
                    )
                    .with_param("id", id),
                );
            }
        }
        match input.slug.as_deref().map(str::trim) {
            Some(slug) if !slug.is_empty() => {
                if !autoxide_kernel::slug::is_valid(slug) {
                    problems.push(FieldError::new(
                        "slug",
                        "invalid",
                        "must be lower-case letters and digits joined by `-`",
                    ));
                }
                row.slug = slug.to_owned();
            }
            _ if row.slug.is_empty() => {
                row.slug = autoxide_kernel::slug::from_title(&row.display_name, row.id)
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

/// Xoá tác giả `id` và liên kết với mục; `None` nếu không có. Event `content.author_changed` ghi qua
/// `outbox` — `None` khi module đang tắt (chế độ nghỉ, bước 70 P9): không ai phát event của module
/// tắt, nên không ghi.
pub(crate) async fn remove_author_with(
    tenant: &TenantContext,
    outbox: Option<&Outbox>,
    id: Uuid,
    actor: Actor,
) -> Result<Option<Author>, Error> {
    let transaction = tenant.database().begin_write().await?;
    let Some(before) = author::Entity::find_by_id(id).scoped(tenant).one(&transaction).await?
    else {
        return Ok(None);
    };
    author::Entity::delete_by_id(id).scoped(tenant).exec(&transaction).await?;
    entry_author::Entity::delete_many()
        .scoped(tenant)
        .filter(entry_author::Column::AuthorId.eq(id))
        .exec(&transaction)
        .await?;
    if let Some(outbox) = outbox {
        let change = Change { before: Some(author_summary(&before)), after: None, published: None };
        outbox
            .record(
                &transaction,
                tenant,
                actor,
                events::subject("author", id),
                &events::AuthorChanged(change),
            )
            .await?;
    }
    transaction.commit().await?;
    Ok(Some(before))
}
