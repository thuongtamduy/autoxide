//! Danh mục (cây, `category`) và thẻ (phẳng, `tag`) — có hiệu lực ngay, không qua nháp
//! (D-102). Tên, slug, mô tả theo từng ngôn ngữ (`content_term_translations`); thiếu bản dịch
//! thì hiển thị bằng ngôn ngữ mặc định và ghi rõ ngôn ngữ thật đã trả.

use std::collections::HashMap;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use chrono::{DateTime, Utc};
use sea_orm::sea_query::Query;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseTransaction, EntityTrait,
    IntoActiveModel, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, Set,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::entity::{entry, entry_term, published, term, term_translation};
use crate::events::{self, Change};
use crate::permissions::{CONTENT_MANAGE, CONTENT_READ};
use crate::publishing::Published;
use crate::service::{ContentService, check_length, invalid, now};

pub(crate) type Term = term::Model;
pub(crate) type TermText = term_translation::Model;

const NAME_MAX: usize = 200;
const DESCRIPTION_MAX: usize = 5000;
/// Số term tối đa của một mục.
const MAX_PER_ENTRY: usize = 50;

/// What a term groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Taxonomy {
    /// Categories nest (a parent is allowed).
    Category,
    /// Tags are flat.
    Tag,
}

impl Taxonomy {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Taxonomy::Category => "category",
            Taxonomy::Tag => "tag",
        }
    }
}

/// Một term như hiển thị cùng mục: tên và slug theo ngôn ngữ của mục (thiếu thì ngôn ngữ mặc
/// định — `locale` nói ngôn ngữ thật đã trả).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TermRef {
    pub id: Uuid,
    pub taxonomy: String,
    pub name: String,
    pub slug: String,
    pub locale: String,
}

/// Một term kèm mọi bản dịch tên của nó.
#[derive(Debug, Clone)]
pub(crate) struct TermWithTexts {
    pub term: Term,
    pub texts: Vec<TermText>,
}

/// Thay đổi tên / mô tả của term trong một ngôn ngữ, và cha (danh mục).
#[derive(Debug, Clone, Default)]
pub(crate) struct TermInput {
    pub locale: Option<String>,
    pub name: Option<String>,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    /// `Some(None)`: về gốc.
    pub parent_id: Option<Option<Uuid>>,
}

/// Điều kiện "mục mang term `term_id`", cho bộ lọc danh sách quản trị.
pub(crate) fn has_term(tenant: &TenantContext, term_id: Uuid) -> sea_orm::sea_query::SimpleExpr {
    entry::Column::Id.in_subquery(
        Query::select()
            .column(entry_term::Column::EntryId)
            .from(entry_term::Entity)
            .and_where(entry_term::Column::TenantId.eq(tenant.id().as_uuid()))
            .and_where(entry_term::Column::TermId.eq(term_id))
            .to_owned(),
    )
}

/// Như [`has_term`], cho bảng bản công khai.
pub(crate) fn published_has_term(
    tenant: &TenantContext,
    term_id: Uuid,
) -> sea_orm::sea_query::SimpleExpr {
    published::Column::EntryId.in_subquery(
        Query::select()
            .column(entry_term::Column::EntryId)
            .from(entry_term::Entity)
            .and_where(entry_term::Column::TenantId.eq(tenant.id().as_uuid()))
            .and_where(entry_term::Column::TermId.eq(term_id))
            .to_owned(),
    )
}

/// ID term của từng mục trong `ids`, theo thứ tự gắn.
pub(crate) async fn term_ids_of<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    ids: &[Uuid],
) -> Result<HashMap<Uuid, Vec<Uuid>>, Error> {
    let mut found: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    if ids.is_empty() {
        return Ok(found);
    }
    let links = entry_term::Entity::find()
        .scoped(tenant)
        .filter(entry_term::Column::EntryId.is_in(ids.to_vec()))
        .order_by_asc(entry_term::Column::SortOrder)
        .all(connection)
        .await?;
    for link in links {
        found.entry(link.entry_id).or_default().push(link.term_id);
    }
    Ok(found)
}

impl ContentService {
    /// ID term của từng mục trong `ids`, theo thứ tự gắn (để API trả mục kèm term).
    pub(crate) async fn term_ids(
        &self,
        tenant: &TenantContext,
        ids: &[Uuid],
    ) -> Result<HashMap<Uuid, Vec<Uuid>>, Error> {
        term_ids_of(tenant.database().read(), tenant, ids).await
    }
}

/// Term của từng bản công khai trong `rows` (theo thứ tự gắn), tên theo ngôn ngữ của mục.
pub(crate) async fn terms_of<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    ids: &[Uuid],
    default_locale: &str,
    rows: &[Published],
) -> Result<HashMap<Uuid, Vec<TermRef>>, Error> {
    let links = term_ids_of(connection, tenant, ids).await?;
    let term_ids: Vec<Uuid> = links.values().flatten().copied().collect();
    if term_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let texts = term_translation::Entity::find()
        .scoped(tenant)
        .filter(term_translation::Column::TermId.is_in(term_ids))
        .all(connection)
        .await?;
    let mut found = HashMap::new();
    for row in rows {
        let refs = links
            .get(&row.entry_id)
            .into_iter()
            .flatten()
            .filter_map(|id| pick(&texts, *id, &row.locale, default_locale))
            .collect();
        found.insert(row.entry_id, refs);
    }
    Ok(found)
}

/// Bản dịch tên của term `id` trong `locale`, hay ngôn ngữ mặc định, hay bất kỳ.
fn pick(texts: &[TermText], id: Uuid, locale: &str, default_locale: &str) -> Option<TermRef> {
    let mine: Vec<&TermText> = texts.iter().filter(|text| text.term_id == id).collect();
    let chosen = mine
        .iter()
        .find(|text| text.locale == locale)
        .or_else(|| mine.iter().find(|text| text.locale == default_locale))
        .or_else(|| mine.first())?;
    Some(TermRef {
        id,
        taxonomy: chosen.taxonomy.clone(),
        name: chosen.name.clone(),
        slug: chosen.slug.clone(),
        locale: chosen.locale.clone(),
    })
}

impl ContentService {
    pub(crate) async fn list_terms(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        taxonomy: Option<Taxonomy>,
        search: Option<&str>,
        page: &PageRequest,
    ) -> Result<Page<TermWithTexts>, Error> {
        principal.require(&CONTENT_READ)?;
        let mut select = term::Entity::find().scoped(tenant);
        if let Some(taxonomy) = taxonomy {
            select = select.filter(term::Column::Taxonomy.eq(taxonomy.as_str()));
        }
        if let Some(search) = search.map(str::trim).filter(|search| !search.is_empty()) {
            // Tên nằm ở bảng bản dịch, không có cột tìm dạng bỏ dấu: so trong bộ nhớ bằng
            // `search::fold` (cùng cách tìm bài) — danh mục và thẻ của một khách không nhiều.
            let wanted = autoxide_kernel::search::fold(search);
            let names: Vec<(Uuid, String)> = term_translation::Entity::find()
                .scoped(tenant)
                .select_only()
                .column(term_translation::Column::TermId)
                .column(term_translation::Column::Name)
                .into_tuple()
                .all(tenant.database().read())
                .await?;
            let mut ids: Vec<Uuid> = names
                .into_iter()
                .filter(|(_, name)| autoxide_kernel::search::fold(name).contains(&wanted))
                .map(|(id, _)| id)
                .collect();
            ids.sort_unstable();
            ids.dedup();
            select = select.filter(term::Column::Id.is_in(ids));
        }
        let page =
            fetch_page(select, term::Column::Id, |row| row.id, page, tenant.database().read())
                .await?;
        let ids: Vec<Uuid> = page.data.iter().map(|term| term.id).collect();
        let mut texts = self.texts_of(tenant, &ids).await?;
        Ok(page
            .map(|term| TermWithTexts { texts: texts.remove(&term.id).unwrap_or_default(), term }))
    }

    /// Số bài (mọi trạng thái) gắn với từng term trong `ids` — một câu cho cả trang.
    pub(crate) async fn term_entry_counts(
        &self,
        tenant: &TenantContext,
        ids: &[Uuid],
    ) -> Result<HashMap<Uuid, u64>, Error> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let rows: Vec<(Uuid, i64)> = entry_term::Entity::find()
            .scoped(tenant)
            .select_only()
            .column(entry_term::Column::TermId)
            .column_as(entry_term::Column::EntryId.count(), "entries")
            .filter(entry_term::Column::TermId.is_in(ids.iter().copied()))
            .group_by(entry_term::Column::TermId)
            .into_tuple()
            .all(tenant.database().read())
            .await?;
        Ok(rows.into_iter().map(|(id, count)| (id, u64::try_from(count).unwrap_or(0))).collect())
    }

    pub(crate) async fn get_term(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<TermWithTexts, Error> {
        principal.require(&CONTENT_READ)?;
        self.term_with_texts(tenant, id).await
    }

    /// Tạo term với tên trong một ngôn ngữ. Cần `content.manage`.
    pub(crate) async fn create_term(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        taxonomy: Taxonomy,
        input: TermInput,
    ) -> Result<TermWithTexts, Error> {
        principal.require(&CONTENT_MANAGE)?;
        let mut problems = Vec::new();
        let locale = self.check_locale(input.locale.as_deref(), "locale", &mut problems);
        if input.name.as_deref().is_none_or(|name| name.trim().is_empty()) {
            problems.push(
                FieldError::new("name", "required", "must be 1 to {name_max} characters")
                    .with_param("name_max", NAME_MAX),
            );
        }
        let parent = input.parent_id.flatten();
        self.check_term_parent(
            tenant,
            tenant.database().read(),
            taxonomy,
            None,
            parent,
            &mut problems,
        )
        .await?;
        let id = Uuid::now_v7();
        let text = Self::term_text(tenant, id, taxonomy, &locale, None, &input, &mut problems)?;
        if !problems.is_empty() {
            return Err(invalid(problems));
        }
        let now = now();
        let transaction = tenant.database().begin_write().await?;
        self.recheck_term_parent(tenant, &transaction, taxonomy, None, parent).await?;
        let row = term::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant.id().as_uuid()),
            taxonomy: Set(taxonomy.as_str().to_owned()),
            parent_id: Set(parent),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&transaction)
        .await?;
        let text = text
            .into_active_model()
            .insert(&transaction)
            .await
            .map_err(|error| slug_conflict(error, &input))?;
        let change = Change {
            before: None,
            after: Some(term_summary(&row, std::slice::from_ref(&text))),
            published: None,
        };
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                events::subject("term", id),
                &events::TermChanged(change),
            )
            .await?;
        transaction.commit().await?;
        Ok(TermWithTexts { term: row, texts: vec![text] })
    }

    /// Đổi tên / mô tả trong một ngôn ngữ (thêm nếu chưa có) và cha. Cần `content.manage`.
    pub(crate) async fn update_term(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        input: TermInput,
        expected: Option<DateTime<Utc>>,
    ) -> Result<TermWithTexts, Error> {
        principal.require(&CONTENT_MANAGE)?;
        let before = self.term_with_texts(tenant, id).await?;
        let taxonomy =
            if before.term.taxonomy == "category" { Taxonomy::Category } else { Taxonomy::Tag };
        let mut problems = Vec::new();
        if let Some(parent) = input.parent_id {
            self.check_term_parent(
                tenant,
                tenant.database().read(),
                taxonomy,
                Some(id),
                parent,
                &mut problems,
            )
            .await?;
        }
        let text = match &input.locale {
            None if input.name.is_some() || input.slug.is_some() || input.description.is_some() => {
                problems.push(FieldError::new(
                    "locale",
                    "required",
                    "say which language the name is in",
                ));
                None
            }
            None => None,
            Some(raw) => {
                let locale = self.check_locale(Some(raw), "locale", &mut problems);
                let existing = before.texts.iter().find(|text| text.locale == locale).cloned();
                if existing.is_none()
                    && input.name.as_deref().is_none_or(|name| name.trim().is_empty())
                {
                    problems.push(FieldError::new(
                        "name",
                        "required",
                        "a new language needs a name",
                    ));
                }
                Some(Self::term_text(
                    tenant,
                    id,
                    taxonomy,
                    &locale,
                    existing,
                    &input,
                    &mut problems,
                )?)
            }
        };
        if !problems.is_empty() {
            return Err(invalid(problems));
        }
        let now = now();
        let transaction = tenant.database().begin_write().await?;
        if let Some(parent) = input.parent_id {
            self.recheck_term_parent(tenant, &transaction, taxonomy, Some(id), parent).await?;
        }
        let mut row = before.term.clone().into_active_model();
        if let Some(parent) = input.parent_id {
            row.parent_id = Set(parent);
        }
        row.updated_at = Set(now);
        let after = term::Entity::update(row)
            .validate()?
            .scoped(tenant)
            .filter(term::Column::UpdatedAt.eq(expected.unwrap_or(before.term.updated_at)))
            .exec(&transaction)
            .await
            .map_err(|error| match error {
                sea_orm::DbErr::RecordNotUpdated => autoxide_kernel::version::changed(),
                error => error.into(),
            })?;
        if let Some(text) = text {
            term_translation::Entity::delete_many()
                .scoped(tenant)
                .filter(term_translation::Column::TermId.eq(id))
                .filter(term_translation::Column::Locale.eq(text.locale.as_str()))
                .exec(&transaction)
                .await?;
            text.into_active_model()
                .insert(&transaction)
                .await
                .map_err(|error| slug_conflict(error, &input))?;
        }
        let texts = term_translation::Entity::find()
            .scoped(tenant)
            .filter(term_translation::Column::TermId.eq(id))
            .all(&transaction)
            .await?;
        let change = Change {
            before: Some(term_summary(&before.term, &before.texts)),
            after: Some(term_summary(&after, &texts)),
            published: None,
        };
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                events::subject("term", id),
                &events::TermChanged(change),
            )
            .await?;
        transaction.commit().await?;
        Ok(TermWithTexts { term: after, texts })
    }

    /// Xoá term (gỡ khỏi mọi mục). Danh mục còn danh mục con thì không được. Cần `content.manage`.
    pub(crate) async fn delete_term(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        principal.require(&CONTENT_MANAGE)?;
        let before = self.term_with_texts(tenant, id).await?;
        let transaction = tenant.database().begin_write().await?;
        // Cùng khoá với tạo / đổi cha của danh mục: không tạo được danh mục con dưới một danh
        // mục đang bị xoá.
        lock_categories(&transaction, tenant).await?;
        let deleted = term::Entity::delete_by_id(id).scoped(tenant).exec(&transaction).await?;
        if deleted.rows_affected == 0 {
            transaction.rollback().await?;
            return Err(term_not_found(id));
        }
        let children = term::Entity::find()
            .scoped(tenant)
            .filter(term::Column::ParentId.eq(id))
            .count(&transaction)
            .await?;
        if children > 0 {
            transaction.rollback().await?;
            return Err(Error::conflict(
                "content.term_has_children",
                "The category still has {count} subcategories.",
            )
            .with_param("count", children));
        }
        term_translation::Entity::delete_many()
            .scoped(tenant)
            .filter(term_translation::Column::TermId.eq(id))
            .exec(&transaction)
            .await?;
        entry_term::Entity::delete_many()
            .scoped(tenant)
            .filter(entry_term::Column::TermId.eq(id))
            .exec(&transaction)
            .await?;
        let change = Change {
            before: Some(term_summary(&before.term, &before.texts)),
            after: None,
            published: None,
        };
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                events::subject("term", id),
                &events::TermChanged(change),
            )
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Thay bộ term của mục `entry_id` (theo thứ tự; term đầu là chính). Có hiệu lực ngay, cả
    /// với bản công khai. Cần `content.write` (hay `content.write_own` trên mục của mình; không
    /// cần `content.read`, như mọi thao tác sửa mục — D-111), và quyền đăng khi mục đang công
    /// khai.
    pub(crate) async fn set_entry_terms(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        entry_id: Uuid,
        term_ids: Vec<Uuid>,
    ) -> Result<Vec<Uuid>, Error> {
        crate::permissions::require_create(principal)?;
        let entry = self.load_checked(tenant, entry_id, None).await?;
        crate::permissions::require_write(principal, &entry)?;
        let mut ids: Vec<Uuid> = Vec::new();
        for id in term_ids {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        if ids.len() > MAX_PER_ENTRY {
            return Err(invalid(vec![
                FieldError::new("term_ids", "length", "at most {max_per_entry} terms")
                    .with_param("max_per_entry", MAX_PER_ENTRY),
            ]));
        }
        let transaction = tenant.database().begin_write().await?;
        // Khoá dòng mục trước khi xoá rồi chèn lại liên kết: hai lệnh gắn cùng lúc chạy lần lượt
        // (không vấp unique thành lỗi 500 trên Postgres), và lệnh xoá mục chạy cùng lúc không để
        // lại liên kết mồ côi. Trạng thái (cần quyền đăng không) đọc lại sau khi khoá.
        let entry = crate::entries::lock_entry(&transaction, tenant, entry.id).await?;
        if crate::entries::Status::of(&entry) == crate::entries::Status::Published {
            // Gắn có hiệu lực ngay cả trên bản công khai: như mọi thay đổi người đọc thấy, mục
            // đang công khai thì cần quyền đăng.
            crate::permissions::require_publish(principal, &entry)?;
        }
        let known = term::Entity::find()
            .scoped(tenant)
            .filter(term::Column::Id.is_in(ids.clone()))
            .count(&transaction)
            .await?;
        if known != ids.len() as u64 {
            return Err(invalid(vec![FieldError::new(
                "term_ids",
                "not_found",
                "every id must be an existing category or tag",
            )]));
        }
        let before = term_ids_of(&transaction, tenant, &[entry_id])
            .await?
            .remove(&entry_id)
            .unwrap_or_default();
        entry_term::Entity::delete_many()
            .scoped(tenant)
            .filter(entry_term::Column::EntryId.eq(entry_id))
            .exec(&transaction)
            .await?;
        for (order, term_id) in ids.iter().enumerate() {
            entry_term::ActiveModel {
                tenant_id: Set(tenant.id().as_uuid()),
                entry_id: Set(entry_id),
                term_id: Set(*term_id),
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
            LinkChange { field: "term_ids", before, after: ids.clone() },
        )
        .await?;
        transaction.commit().await?;
        Ok(ids)
    }

    /// Event "mục đổi" khi term / tác giả đổi; mục đang công khai thì kèm dữ liệu làm mới cache.
    pub(crate) async fn record_link_change<C: ConnectionTrait>(
        &self,
        tenant: &TenantContext,
        transaction: &C,
        principal: &Principal,
        entry: &crate::entries::Entry,
        link: LinkChange<'_>,
    ) -> Result<(), Error> {
        let LinkChange { field, before, after } = link;
        let path = published::Entity::find()
            .scoped(tenant)
            .filter(published::Column::EntryId.eq(entry.id))
            .one(transaction)
            .await?
            .map(|row| row.path);
        let change = Change {
            before: Some(json!({ field: before })),
            after: Some(json!({ field: after })),
            published: path
                .as_deref()
                .map(|path| crate::publishing::published_payload(entry, Some(path), &[])),
        };
        self.outbox
            .record(
                transaction,
                tenant,
                Actor::from(principal),
                events::entry_subject(entry.id),
                &events::EntryUpdated(change),
            )
            .await?;
        Ok(())
    }

    /// Danh mục / thẻ đang có ít nhất một bài đã đăng ở `locale`, mỗi cái kèm số bài đó (cho
    /// trang blog công khai). Một trang theo ID; số bài đếm bằng một câu `GROUP BY` cho cả trang.
    pub(crate) async fn public_terms(
        &self,
        tenant: &TenantContext,
        locale: &str,
        taxonomy: Option<Taxonomy>,
        page: &PageRequest,
    ) -> Result<Page<(TermRef, u64)>, Error> {
        let mut problems = Vec::new();
        let locale = self.check_locale(Some(locale), "locale", &mut problems);
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let posts = || {
            Query::select()
                .column(published::Column::EntryId)
                .from(published::Entity)
                .and_where(published::Column::TenantId.eq(tenant.id().as_uuid()))
                .and_where(published::Column::Kind.eq("post"))
                .and_where(published::Column::Locale.eq(locale.as_str()))
                .to_owned()
        };
        let used = Query::select()
            .column(entry_term::Column::TermId)
            .from(entry_term::Entity)
            .and_where(entry_term::Column::TenantId.eq(tenant.id().as_uuid()))
            .and_where(entry_term::Column::EntryId.in_subquery(posts()))
            .to_owned();
        let mut select =
            term::Entity::find().scoped(tenant).filter(term::Column::Id.in_subquery(used));
        if let Some(taxonomy) = taxonomy {
            select = select.filter(term::Column::Taxonomy.eq(taxonomy.as_str()));
        }
        let found =
            fetch_page(select, term::Column::Id, |row| row.id, page, tenant.database().read())
                .await?;
        let ids: Vec<Uuid> = found.data.iter().map(|term| term.id).collect();
        let counts: HashMap<Uuid, i64> = if ids.is_empty() {
            HashMap::new()
        } else {
            entry_term::Entity::find()
                .scoped(tenant)
                .select_only()
                .column(entry_term::Column::TermId)
                .column_as(entry_term::Column::EntryId.count(), "posts")
                .filter(entry_term::Column::TermId.is_in(ids.iter().copied()))
                .filter(entry_term::Column::EntryId.in_subquery(posts()))
                .group_by(entry_term::Column::TermId)
                .into_tuple::<(Uuid, i64)>()
                .all(tenant.database().read())
                .await?
                .into_iter()
                .collect()
        };
        let texts: Vec<TermText> =
            self.texts_of(tenant, &ids).await?.into_values().flatten().collect();
        let Page { data, pagination } = found;
        let mut shown = Vec::with_capacity(data.len());
        for term in data {
            let named = pick(&texts, term.id, &locale, &self.config.default_locale)
                .ok_or_else(|| Error::internal("a term without names"))?;
            let count = counts.get(&term.id).copied().unwrap_or(0);
            shown.push((named, u64::try_from(count).unwrap_or(0)));
        }
        Ok(Page { data: shown, pagination })
    }

    /// Term công khai theo slug trong `locale` (thiếu bản dịch thì tìm ở ngôn ngữ mặc định).
    pub(crate) async fn term_by_slug(
        &self,
        tenant: &TenantContext,
        taxonomy: Taxonomy,
        locale: &str,
        slug: &str,
    ) -> Result<(TermWithTexts, TermRef), Error> {
        let mut problems = Vec::new();
        let locale = self.check_locale(Some(locale), "locale", &mut problems);
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let mut found = None;
        for language in [locale.as_str(), self.config.default_locale.as_str()] {
            found = term_translation::Entity::find()
                .scoped(tenant)
                .filter(term_translation::Column::Taxonomy.eq(taxonomy.as_str()))
                .filter(term_translation::Column::Locale.eq(language))
                .filter(term_translation::Column::Slug.eq(slug))
                .one(tenant.database().read())
                .await?;
            if found.is_some() {
                break;
            }
        }
        let text = found.ok_or_else(|| {
            Error::not_found("content.term_not_found", "No category or tag has the slug `{slug}`.")
                .with_param("slug", slug)
        })?;
        let full = self.term_with_texts(tenant, text.term_id).await?;
        let shown = pick(&full.texts, text.term_id, &locale, &self.config.default_locale)
            .ok_or_else(|| Error::internal("a term without names"))?;
        Ok((full, shown))
    }

    async fn term_with_texts(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<TermWithTexts, Error> {
        let term = term::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| term_not_found(id))?;
        let texts = self.texts_of(tenant, &[id]).await?.remove(&id).unwrap_or_default();
        Ok(TermWithTexts { term, texts })
    }

    async fn texts_of(
        &self,
        tenant: &TenantContext,
        ids: &[Uuid],
    ) -> Result<HashMap<Uuid, Vec<TermText>>, Error> {
        let mut found: HashMap<Uuid, Vec<TermText>> = HashMap::new();
        if ids.is_empty() {
            return Ok(found);
        }
        for text in term_translation::Entity::find()
            .scoped(tenant)
            .filter(term_translation::Column::TermId.is_in(ids.to_vec()))
            .order_by_asc(term_translation::Column::Locale)
            .all(tenant.database().read())
            .await?
        {
            found.entry(text.term_id).or_default().push(text);
        }
        Ok(found)
    }

    /// Kiểm lại cha của danh mục TRONG transaction ghi, sau khi khoá cây danh mục: hai lệnh
    /// đổi cha chéo nhau (A vào B, B vào A) không thể cùng qua bước kiểm vòng lặp, và cha không
    /// thể vừa bị xoá. Lần kiểm trước transaction chỉ để gom lỗi báo một lần.
    async fn recheck_term_parent(
        &self,
        tenant: &TenantContext,
        transaction: &DatabaseTransaction,
        taxonomy: Taxonomy,
        id: Option<Uuid>,
        parent: Option<Uuid>,
    ) -> Result<(), Error> {
        if parent.is_none() {
            return Ok(());
        }
        lock_categories(transaction, tenant).await?;
        let mut problems = Vec::new();
        self.check_term_parent(tenant, transaction, taxonomy, id, parent, &mut problems).await?;
        if problems.is_empty() { Ok(()) } else { Err(invalid(problems)) }
    }

    /// Cha của danh mục: là danh mục, có thật, không tạo vòng. Thẻ không có cha.
    async fn check_term_parent<C: ConnectionTrait>(
        &self,
        tenant: &TenantContext,
        connection: &C,
        taxonomy: Taxonomy,
        id: Option<Uuid>,
        parent: Option<Uuid>,
        problems: &mut Vec<FieldError>,
    ) -> Result<(), Error> {
        let Some(parent) = parent else { return Ok(()) };
        if taxonomy == Taxonomy::Tag {
            problems.push(FieldError::new(
                "parent_id",
                "not_allowed",
                "tags are flat; only categories have a parent",
            ));
            return Ok(());
        }
        let mut current = Some(parent);
        for _ in 0..100 {
            let Some(here) = current else { return Ok(()) };
            if Some(here) == id {
                problems.push(FieldError::new(
                    "parent_id",
                    "term_cycle",
                    "a category cannot go under itself or a subcategory",
                ));
                return Ok(());
            }
            match term::Entity::find_by_id(here).scoped(tenant).one(connection).await? {
                Some(row) if row.taxonomy == "category" => current = row.parent_id,
                _ => {
                    problems.push(FieldError::new(
                        "parent_id",
                        "not_found",
                        "no category has this id",
                    ));
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    /// Dòng tên của term trong `locale`, từ `existing` (nếu có) cộng `input`; kiểm mọi trường.
    fn term_text(
        tenant: &TenantContext,
        id: Uuid,
        taxonomy: Taxonomy,
        locale: &str,
        existing: Option<TermText>,
        input: &TermInput,
        problems: &mut Vec<FieldError>,
    ) -> Result<TermText, Error> {
        let mut text = existing.unwrap_or_else(|| TermText {
            tenant_id: tenant.id().as_uuid(),
            term_id: id,
            locale: locale.to_owned(),
            taxonomy: taxonomy.as_str().to_owned(),
            name: String::new(),
            slug: String::new(),
            description: String::new(),
            meta_title: String::new(),
            meta_description: String::new(),
        });
        check_length("name", input.name.as_deref(), NAME_MAX, problems);
        check_length("description", input.description.as_deref(), DESCRIPTION_MAX, problems);
        check_length("meta_title", input.meta_title.as_deref(), 300, problems);
        check_length("meta_description", input.meta_description.as_deref(), 1000, problems);
        if let Some(name) = &input.name {
            text.name = name.trim().to_owned();
        }
        for (target, value) in [
            (&mut text.description, &input.description),
            (&mut text.meta_title, &input.meta_title),
            (&mut text.meta_description, &input.meta_description),
        ] {
            if let Some(value) = value {
                *target = value.trim().to_owned();
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
                text.slug = slug.to_owned();
            }
            _ if text.slug.is_empty() => {
                text.slug = crate::entries::slug::from_title(&text.name, id)
            }
            _ => {}
        }
        Ok(text)
    }
}

fn term_summary(term: &Term, texts: &[TermText]) -> Value {
    json!({
        "taxonomy": term.taxonomy,
        "parent_id": term.parent_id,
        "names": texts.iter().map(|text| json!({"locale": text.locale, "name": text.name, "slug": text.slug})).collect::<Vec<_>>(),
    })
}

fn slug_conflict(error: sea_orm::DbErr, input: &TermInput) -> Error {
    if crate::entries::is_unique_violation(&error) {
        Error::conflict(
            "content.slug_taken",
            "Another category or tag already uses this slug in this language.",
        )
        .with_param("slug", input.slug.clone().unwrap_or_default())
    } else {
        error.into()
    }
}

pub(crate) fn term_not_found(id: Uuid) -> Error {
    Error::not_found("content.term_not_found", "No category or tag has the id `{id}`.")
        .with_param("id", id)
}

/// Danh sách liên kết của một mục đổi (term hay tác giả): tên trường, trước, sau.
pub(crate) struct LinkChange<'a> {
    pub field: &'a str,
    pub before: Vec<Uuid>,
    pub after: Vec<Uuid>,
}

/// Khoá chung cho cây danh mục (mọi ngôn ngữ dùng một cây): dùng lại bảng khoá cây trang với
/// một khoá không thể trùng mã ngôn ngữ BCP 47.
async fn lock_categories(
    transaction: &DatabaseTransaction,
    tenant: &TenantContext,
) -> Result<(), Error> {
    crate::publishing::paths::lock_tree(transaction, tenant, "#categories").await
}
