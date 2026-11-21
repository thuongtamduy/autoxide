//! Nhãn (tag) của item, song song với thư mục (D-094): một item nằm trong đúng một thư mục
//! nhưng mang được nhiều nhãn.
//!
//! - Nhãn được tạo khi lần đầu gắn cho một item; tên không trùng (không phân biệt hoa
//!   thường), 1 tới 50 ký tự, không có `,` (dấu phẩy ngăn các nhãn khi tải lên).
//! - Gắn nhãn là thay cả bộ nhãn của item, trong cùng transaction với lệnh sửa item.
//! - Đúng khi chạy đồng thời trên cả hai database: tạo nhãn bằng `INSERT … ON CONFLICT DO
//!   NOTHING` rồi đọc lại; trước khi gắn thì "chạm" dòng nhãn (khoá tới hết transaction),
//!   lệnh xoá nhãn xoá dòng trước rồi mới gỡ khỏi item — không còn liên kết tới nhãn đã xoá.

use autoxide_kernel::time::Timestamp;
use std::collections::HashMap;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, SortValue, fetch_page, fetch_page_by};
use autoxide_kernel::rows;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::sea_query::{OnConflict, Query};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter, QuerySelect, Set,
    SqlErr,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::entity::{item, item_tag, tag};
use crate::events::{self, Change};
use crate::permissions::{MEDIA_READ, MEDIA_WRITE};
use crate::service::{MediaService, now};

/// Một nhãn.
pub(crate) type Tag = tag::Model;

/// Số nhãn tối đa của một item.
pub(crate) const MAX_TAGS_PER_ITEM: usize = 20;
/// Độ dài tối đa của tên nhãn, theo ký tự.
pub(crate) const TAG_MAX_LENGTH: usize = 50;

// Sắp danh sách nhãn theo gì.
/// Field to sort the tag list by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TagSort {
    /// Name, ignoring case (the default).
    #[default]
    Name,
    /// Creation time.
    CreatedAt,
}

/// Một nhãn kèm số item (ngoài thùng rác) mang nó.
#[derive(Debug, Clone)]
pub(crate) struct TagWithCount {
    pub tag: Tag,
    pub items: u64,
}

/// Kiểm một bộ tên nhãn (thêm lỗi vào `problems` dưới tên trường `field`); trả các tên đã
/// bỏ khoảng trắng hai đầu, bỏ trùng (không phân biệt hoa thường), giữ thứ tự.
pub(crate) fn check_tags(
    names: &[String],
    field: &str,
    problems: &mut Vec<FieldError>,
) -> Vec<String> {
    let mut kept: Vec<String> = Vec::new();
    let mut invalid = false;
    for name in names {
        let name = name.trim();
        let length = name.chars().count();
        if !(1..=TAG_MAX_LENGTH).contains(&length)
            || name.chars().any(|c| c == ',' || c.is_control())
        {
            invalid = true;
            continue;
        }
        let key = autoxide_kernel::search::normalize(name);
        if !kept.iter().any(|existing| autoxide_kernel::search::normalize(existing) == key) {
            kept.push(name.to_owned());
        }
    }
    if invalid {
        problems.push(FieldError::new(
            field,
            "invalid",
            "each tag must be 1 to {tag_max_length} characters, without `,` or control characters",
        ).with_param("tag_max_length", TAG_MAX_LENGTH));
    }
    if kept.len() > MAX_TAGS_PER_ITEM {
        problems.push(
            FieldError::new(field, "length", "at most {max_tags_per_item} tags per item")
                .with_param("max_tags_per_item", MAX_TAGS_PER_ITEM),
        );
    }
    kept
}

/// Đặt bộ nhãn của item `item_id` thành `names` (đã qua [`check_tags`]), trong `transaction`.
/// Nhãn chưa có thì tạo. Trả tên các nhãn, như đã lưu, theo thứ tự tên.
pub(crate) async fn set_item_tags<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    item_id: Uuid,
    names: &[String],
) -> Result<Vec<String>, Error> {
    let mut ids = Vec::with_capacity(names.len());
    for name in names {
        ids.push(ensure_tag(transaction, tenant, name).await?);
    }
    let mut stale =
        item_tag::Entity::delete_many().scoped(tenant).filter(item_tag::Column::ItemId.eq(item_id));
    if !ids.is_empty() {
        stale = stale.filter(item_tag::Column::TagId.is_not_in(ids.clone()));
    }
    stale.exec(transaction).await?;
    if !ids.is_empty() {
        let links = ids.iter().map(|tag_id| item_tag::ActiveModel {
            tenant_id: Set(tenant.id().as_uuid()),
            item_id: Set(item_id),
            tag_id: Set(*tag_id),
        });
        item_tag::Entity::insert_many(links)
            .on_conflict(
                OnConflict::columns([
                    item_tag::Column::TenantId,
                    item_tag::Column::ItemId,
                    item_tag::Column::TagId,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec_without_returning(transaction)
            .await?;
    }
    Ok(tags_of(transaction, tenant, &[item_id]).await?.remove(&item_id).unwrap_or_default())
}

/// ID của nhãn `name`, tạo nếu chưa có; dòng nhãn bị khoá tới hết `transaction`.
async fn ensure_tag<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    name: &str,
) -> Result<Uuid, Error> {
    let key = autoxide_kernel::search::normalize(name);
    // Vài lần: lệnh xoá nhãn có thể chen vào giữa lúc tạo và lúc khoá.
    for _ in 0..3 {
        let now = now();
        let row = tag::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            name: Set(name.to_owned()),
            name_key: Set(key.clone()),
            search_key: Set(autoxide_kernel::search::fold(name)),
            created_at: Set(now),
            updated_at: Set(now),
        };
        tag::Entity::insert(row)
            .on_conflict(
                OnConflict::columns([tag::Column::TenantId, tag::Column::NameKey])
                    .do_nothing()
                    .to_owned(),
            )
            .exec_without_returning(transaction)
            .await?;
        let Some(found) = tag::Entity::find()
            .scoped(tenant)
            .filter(tag::Column::NameKey.eq(key.as_str()))
            .one(transaction)
            .await?
        else {
            continue;
        };
        if rows::lock_one(transaction, tag::Entity::find_by_id(found.id).scoped(tenant))
            .await?
            .is_some()
        {
            return Ok(found.id);
        }
    }
    Err(Error::internal(format!("the media tag `{name}` keeps disappearing while being assigned")))
}

/// Tên nhãn của từng item trong `ids` (theo thứ tự tên); item không nhãn thì không có mặt.
/// Hai câu query cho mọi item, không N+1.
pub(crate) async fn tags_of<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    ids: &[Uuid],
) -> Result<HashMap<Uuid, Vec<String>>, Error> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let links = item_tag::Entity::find()
        .scoped(tenant)
        .filter(item_tag::Column::ItemId.is_in(ids.to_vec()))
        .all(connection)
        .await?;
    let tag_ids: Vec<Uuid> = links.iter().map(|link| link.tag_id).collect();
    let tags: HashMap<Uuid, Tag> = tag::Entity::find()
        .scoped(tenant)
        .filter(tag::Column::Id.is_in(tag_ids))
        .all(connection)
        .await?
        .into_iter()
        .map(|tag| (tag.id, tag))
        .collect();
    let mut by_item: HashMap<Uuid, Vec<&Tag>> = HashMap::new();
    for link in &links {
        if let Some(tag) = tags.get(&link.tag_id) {
            by_item.entry(link.item_id).or_default().push(tag);
        }
    }
    Ok(by_item
        .into_iter()
        .map(|(item, mut tags)| {
            tags.sort_by(|a, b| a.name_key.cmp(&b.name_key));
            (item, tags.into_iter().map(|tag| tag.name.clone()).collect())
        })
        .collect())
}

/// ID của nhãn tên `name` (không phân biệt hoa thường), nếu có.
pub(crate) async fn find_tag<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    name: &str,
) -> Result<Option<Uuid>, Error> {
    let key = autoxide_kernel::search::normalize(name);
    Ok(tag::Entity::find()
        .scoped(tenant)
        .filter(tag::Column::NameKey.eq(key))
        .one(connection)
        .await?
        .map(|tag| tag.id))
}

/// Điều kiện "item mang nhãn `tag_id`", cho bộ lọc của danh sách.
pub(crate) fn has_tag(tenant: &TenantContext, tag_id: Uuid) -> sea_orm::sea_query::SimpleExpr {
    item::Column::Id.in_subquery(
        Query::select()
            .column(item_tag::Column::ItemId)
            .from(item_tag::Entity)
            .and_where(item_tag::Column::TenantId.eq(tenant.id().as_uuid()))
            .and_where(item_tag::Column::TagId.eq(tag_id))
            .to_owned(),
    )
}

impl MediaService {
    /// Một trang nhãn, kèm số item mang mỗi nhãn. Cần `media.read`.
    pub(crate) async fn list_tags(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        search: Option<&str>,
        sort: TagSort,
        page: &PageRequest,
    ) -> Result<Page<TagWithCount>, Error> {
        principal.require(&MEDIA_READ)?;
        let mut select = tag::Entity::find().scoped(tenant);
        if let Some(search) = search.filter(|search| !search.trim().is_empty()) {
            select =
                select.filter(autoxide_kernel::search::contains(tag::Column::SearchKey, search));
        }
        let read = tenant.database().read();
        let page = match sort {
            TagSort::CreatedAt => {
                fetch_page(select, tag::Column::Id, |row| row.id, page, read).await?
            }
            TagSort::Name => {
                let key = |row: &Tag| SortValue::Text(row.name_key.clone());
                fetch_page_by(
                    select,
                    tag::Column::NameKey,
                    key,
                    tag::Column::Id,
                    |row| row.id,
                    page,
                    read,
                )
                .await?
            }
        };
        let counts = self
            .count_items(tenant, &page.data.iter().map(|tag| tag.id).collect::<Vec<_>>())
            .await?;
        Ok(page.map(|tag| TagWithCount { items: counts.get(&tag.id).copied().unwrap_or(0), tag }))
    }

    /// Nhãn `id`, kèm số item. Cần `media.read`.
    pub(crate) async fn get_tag(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<TagWithCount, Error> {
        principal.require(&MEDIA_READ)?;
        let tag = tag::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| tag_not_found(id))?;
        let items = self.count_items(tenant, &[id]).await?.get(&id).copied().unwrap_or(0);
        Ok(TagWithCount { tag, items })
    }

    /// Đổi tên nhãn `id` (mọi item mang nó thấy tên mới). Cần `media.write`.
    pub(crate) async fn rename_tag(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        name: &str,
        expected: Option<Timestamp>,
    ) -> Result<TagWithCount, Error> {
        principal.require(&MEDIA_WRITE)?;
        let mut problems = Vec::new();
        let names = check_tags(&[name.to_owned()], "name", &mut problems);
        let Some(name) = names.into_iter().next().filter(|_| problems.is_empty()) else {
            return Err(Error::validation(problems));
        };
        let transaction = tenant.database().begin_write().await?;
        let before = tag::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| tag_not_found(id))?;
        let mut row = before.clone().into_active_model();
        row.name_key = Set(autoxide_kernel::search::normalize(&name));
        row.search_key = Set(autoxide_kernel::search::fold(&name));
        row.name = Set(name.clone());
        row.updated_at = Set(now());
        let mut update = tag::Entity::update(row).validate()?.scoped(tenant);
        if let Some(expected) = expected {
            update = update.filter(tag::Column::UpdatedAt.eq(expected));
        }
        let after = match update.exec(&transaction).await {
            Ok(after) => after,
            Err(sea_orm::DbErr::RecordNotUpdated) => {
                return Err(autoxide_kernel::version::changed());
            }
            Err(error) => return Err(name_conflict(error, &name)),
        };
        let change = Change {
            before: Some(json!({"name": before.name})),
            after: Some(json!({"name": after.name})),
        };
        let actor = Actor::from(principal);
        self.outbox
            .record(
                &transaction,
                tenant,
                actor,
                events::tag_subject(id),
                &events::TagUpdated(change),
            )
            .await?;
        transaction.commit().await?;
        let items = self.count_items(tenant, &[id]).await?.get(&id).copied().unwrap_or(0);
        Ok(TagWithCount { tag: after, items })
    }

    /// Xoá nhãn `id` và gỡ nó khỏi mọi item. Cần `media.write`.
    pub(crate) async fn delete_tag(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        principal.require(&MEDIA_WRITE)?;
        let transaction = tenant.database().begin_write().await?;
        let tag = tag::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| tag_not_found(id))?;
        // Xoá dòng nhãn TRƯỚC (khoá nó), rồi mới gỡ khỏi item (xem đầu file).
        let deleted = tag::Entity::delete_by_id(id).scoped(tenant).exec(&transaction).await?;
        if deleted.rows_affected == 0 {
            transaction.rollback().await?;
            return Err(tag_not_found(id));
        }
        let unlinked = item_tag::Entity::delete_many()
            .scoped(tenant)
            .filter(item_tag::Column::TagId.eq(id))
            .exec(&transaction)
            .await?;
        let before = json!({"name": tag.name, "items": unlinked.rows_affected});
        let change = Change { before: Some(before), after: None };
        let actor = Actor::from(principal);
        self.outbox
            .record(
                &transaction,
                tenant,
                actor,
                events::tag_subject(id),
                &events::TagDeleted(change),
            )
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Số item ngoài thùng rác mang từng nhãn trong `ids`.
    async fn count_items(
        &self,
        tenant: &TenantContext,
        ids: &[Uuid],
    ) -> Result<HashMap<Uuid, u64>, Error> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let live = Query::select()
            .column(item::Column::Id)
            .from(item::Entity)
            .and_where(item::Column::TenantId.eq(tenant.id().as_uuid()))
            .and_where(item::Column::TrashedAt.is_null())
            .to_owned();
        let rows: Vec<(Uuid, i64)> = item_tag::Entity::find()
            .scoped(tenant)
            .select_only()
            .column(item_tag::Column::TagId)
            .column_as(item_tag::Column::ItemId.count(), "items")
            .filter(item_tag::Column::TagId.is_in(ids.to_vec()))
            .filter(item_tag::Column::ItemId.in_subquery(live))
            .group_by(item_tag::Column::TagId)
            .into_tuple()
            .all(tenant.database().read())
            .await?;
        Ok(rows.into_iter().map(|(id, count)| (id, u64::try_from(count).unwrap_or(0))).collect())
    }
}

fn name_conflict(error: sea_orm::DbErr, name: &str) -> Error {
    match error.sql_err() {
        Some(SqlErr::UniqueConstraintViolation(_)) => Error::conflict(
            "media.tag_name_taken",
            "A tag named `{name}` already exists (names ignore case). Choose another name.",
        )
        .with_param("name", name),
        _ => Error::from(error),
    }
}

pub(crate) fn tag_not_found(id: Uuid) -> Error {
    Error::not_found("media.tag_not_found", "Media tag `{id}` does not exist.").with_param("id", id)
}
