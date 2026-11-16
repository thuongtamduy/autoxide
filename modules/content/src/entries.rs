//! Bản nháp của bài viết và trang: tạo, đọc, liệt kê, sửa. Mỗi bản dịch là một mục riêng
//! (D-102). Đăng, gỡ, chuyển, xoá ở `publishing.rs`.

pub(crate) mod body;
pub(crate) mod review;
pub(crate) mod revisions;
pub(crate) mod slug;

use std::collections::BTreeSet;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, SortValue, fetch_page, fetch_page_by};
use autoxide_kernel::rows;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use chrono::{DateTime, Utc};
use sea_orm::sea_query::ExprTrait as _;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter,
    QuerySelect, Set, SqlErr,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::entity::ROOT;
use crate::entity::entry::{self, Column, Entity};
use crate::entries::revisions::Reason;
use crate::events::{self, Change};
use crate::permissions::{self, CONTENT_READ};
use crate::service::{ContentService, check_length, check_url, entry_not_found, invalid, now};

/// Số lần thử tạo mục với slug tự sinh. Mỗi lượt chỉ một trong các request tạo CÙNG slug lúc đó
/// thắng (Postgres chạy chúng song song; SQLite chỉ một bên ghi nên nối đuôi): đủ cho chừng ấy
/// request trùng tiêu đề cùng lúc mà không request nào nhận `content.slug_taken` oan.
const SLUG_ATTEMPTS: usize = 12;

/// Một mục (bản nháp làm việc).
pub(crate) type Entry = entry::Model;

/// Độ sâu tối đa của cây trang (trang gốc là cấp 1).
pub(crate) const MAX_TREE_DEPTH: usize = 5;
/// Sửa cách revision gần nhất quá chừng này phút thì tự lưu một revision.
const BACKGROUND_SAVE_MINUTES: i64 = 10;
const TITLE_MAX: usize = 300;
const EXCERPT_MAX: usize = 1000;
const ALT_MAX: usize = 1000;
const META_TITLE_MAX: usize = 300;
const META_DESCRIPTION_MAX: usize = 1000;

/// What an entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Kind {
    /// A dated article, at `/<post prefix>/<slug>`; grouped by categories and tags.
    Post,
    /// A page, at `/<parent slugs>/<slug>`; pages nest.
    Page,
}

impl Kind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Kind::Post => "post",
            Kind::Page => "page",
        }
    }

    pub(crate) fn parse(raw: &str) -> Option<Kind> {
        match raw {
            "post" => Some(Kind::Post),
            "page" => Some(Kind::Page),
            _ => None,
        }
    }
}

/// Where an entry is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Status {
    /// Never published, or taken down: readers do not see it.
    Draft,
    /// Readers see the published version (the draft may have newer changes).
    Published,
    /// Taken down for good: its address answers 410.
    Archived,
}

impl Status {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Status::Draft => "draft",
            Status::Published => "published",
            Status::Archived => "archived",
        }
    }

    pub(crate) fn of(entry: &Entry) -> Status {
        match entry.status.as_str() {
            "published" => Status::Published,
            "archived" => Status::Archived,
            _ => Status::Draft,
        }
    }
}

/// Các trường sửa được; `None` giữ nguyên (khi tạo: giá trị mặc định).
#[derive(Debug, Clone, Default)]
pub(crate) struct EntryFields {
    pub title: Option<String>,
    pub slug: Option<String>,
    pub excerpt: Option<String>,
    pub body: Option<Value>,
    /// `Some(None)`: bỏ ảnh.
    pub featured_media_id: Option<Option<Uuid>>,
    pub featured_media_alt: Option<String>,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub og_media_id: Option<Option<Uuid>>,
    pub canonical_url: Option<String>,
    pub noindex: Option<bool>,
    /// Nội dung do AI tạo (D-109). Agent nên luôn đặt `true`.
    pub ai_generated: Option<bool>,
    /// Chỉ máy chủ đặt (khi tự dịch): revision của bản gốc đã dịch. Không có trong API.
    pub translated_from: Option<Uuid>,
}

/// Một mục mới.
#[derive(Debug, Clone)]
pub(crate) struct NewEntry {
    pub kind: Kind,
    /// `None`: ngôn ngữ mặc định.
    pub locale: Option<String>,
    /// Trang cha (chỉ trang).
    pub parent_id: Option<Uuid>,
    pub fields: EntryFields,
}

/// Một lần sửa bản nháp ghi revision thế nào.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Revise {
    /// Lý do revision bắt buộc ghi (`None`: chỉ ghi khi lần trước đã lâu).
    pub forced: Option<Reason>,
    /// Sửa này là khôi phục revision đó (event `content.entry_restored` thay cho
    /// `content.entry_updated`).
    pub restored_from: Option<Uuid>,
}

/// Trang cha khi lọc danh sách.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ParentFilter {
    #[default]
    Any,
    /// Chỉ mục không có cha (mọi bài, trang gốc).
    Root,
    Id(Uuid),
}

// Sắp danh sách quản trị theo gì.
/// Field to sort the entry list by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EntrySort {
    /// Creation time (the default).
    #[default]
    CreatedAt,
    /// Last change of the draft.
    UpdatedAt,
}

/// Bộ lọc của danh sách quản trị.
#[derive(Debug, Clone, Default)]
pub(crate) struct EntryFilter {
    pub kind: Option<Kind>,
    pub locale: Option<String>,
    pub status: Option<Status>,
    pub parent: ParentFilter,
    pub search: Option<String>,
    pub term_id: Option<Uuid>,
    pub author_id: Option<Uuid>,
    pub translation_group_id: Option<Uuid>,
    pub sort: EntrySort,
    /// Chỉ bài đang (`true`) / không (`false`) chờ duyệt.
    pub review_requested: Option<bool>,
    /// Chỉ bài có (`true`) / không có (`false`) lịch đăng hay gỡ đang chờ.
    pub scheduled: Option<bool>,
    /// Chỉ bài do người này tạo (`created_by=me` đổi thành người gọi ở tầng HTTP).
    pub created_by: Option<Uuid>,
    /// Chỉ bài do AI viết (`true`) / không (`false`).
    pub ai_generated: Option<bool>,
    /// Chỉ bài chưa có bản dịch sang ngôn ngữ này (không mục nào cùng nhóm bản dịch mang nó).
    pub missing_locale: Option<String>,
}

impl ContentService {
    /// Tạo một mục nháp. Cần `content.write` hoặc `content.write_own` (khi đó trang cha phải là
    /// của mình, và mục luôn mang `ai_generated`). Slug để trống thì sinh từ tiêu đề (trùng thì
    /// thêm `-2`, `-3`…).
    pub(crate) async fn create(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewEntry,
    ) -> Result<Entry, Error> {
        permissions::require_create(principal)?;
        if let Some(parent) = input.parent_id {
            // Trang cha không có thì `check_parent` báo bên dưới, cùng các lỗi khác.
            if let Some(row) =
                Entity::find_by_id(parent).scoped(tenant).one(tenant.database().read()).await?
            {
                permissions::require_parent(principal, &row)?;
            }
        }
        let mut problems = Vec::new();
        let locale = self.check_locale(input.locale.as_deref(), "locale", &mut problems);
        if input.fields.title.as_deref().is_none_or(|title| title.trim().is_empty()) {
            problems.push(
                FieldError::new("title", "required", "must be 1 to {title_max} characters")
                    .with_param("title_max", TITLE_MAX),
            );
        }
        if input.parent_id.is_some() && input.kind == Kind::Post {
            problems.push(FieldError::new("parent_id", "not_allowed", "only pages have a parent"));
        }
        let id = Uuid::now_v7();
        let now = now();
        let mut candidate =
            blank(tenant, id, input.kind, &locale, input.parent_id, principal.user_id(), now);
        candidate.created_by_api_key_id = principal.api_key_id();
        let auto_slug = input.fields.slug.is_none();
        self.apply(tenant, &mut candidate, input.fields, &mut problems).await?;
        mark_agent_written(principal, &mut candidate);
        if let Some(parent) = input.parent_id {
            self.check_parent(tenant, tenant.database().read(), &candidate, parent, &mut problems)
                .await?;
        }
        if !problems.is_empty() {
            return Err(invalid(problems));
        }
        self.insert(tenant, principal, candidate, auto_slug, None).await
    }

    /// Ghi `candidate` (đã kiểm) cùng revision `initial` và event. Slug tự sinh mà trùng
    /// (kể cả do một request khác vừa lấy) thì thử hậu tố kế tiếp.
    pub(crate) async fn insert(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        mut candidate: Entry,
        auto_slug: bool,
        source: Option<Uuid>,
    ) -> Result<Entry, Error> {
        let base = candidate.slug.clone();
        for _ in 0..SLUG_ATTEMPTS {
            let transaction = tenant.database().begin_write().await?;
            if let Some(parent) = candidate.parent_id {
                // Khoá cây của ngôn ngữ (cùng khoá với xoá, chuyển, đăng) rồi kiểm lại trang cha
                // trong transaction: không tạo được trang con dưới một trang đang bị xoá, và độ
                // sâu tính trên cây hiện tại (một lệnh `move` vừa xong có thể đã đổi nó; bản dịch
                // lấy cha trong ngôn ngữ khác, có thể đã sâu hơn).
                crate::publishing::paths::lock_tree(&transaction, tenant, &candidate.locale)
                    .await?;
                let mut problems = Vec::new();
                self.check_parent(tenant, &transaction, &candidate, parent, &mut problems).await?;
                if !problems.is_empty() {
                    transaction.rollback().await?;
                    return Err(invalid(problems));
                }
                if !lock(&transaction, tenant, parent).await? {
                    return Err(invalid(vec![FieldError::new(
                        "parent_id",
                        "not_found",
                        "no page has this id",
                    )]));
                }
            }
            if auto_slug {
                candidate.slug = free_slug(&transaction, tenant, &candidate, &base).await?;
            } else if slug_taken(&transaction, tenant, &candidate).await? {
                return Err(slug_conflict(&candidate.slug));
            }
            let inserted = candidate.clone().into_active_model().insert(&transaction).await;
            let entry = match inserted {
                Ok(entry) => entry,
                Err(error) if is_unique_violation(&error) => {
                    transaction.rollback().await?;
                    if self.translation_taken(tenant, &candidate).await? {
                        return Err(Error::conflict(
                            "content.translation_exists",
                            "This entry already has a translation in `{locale}`.",
                        )
                        .with_param("locale", &candidate.locale));
                    }
                    if !auto_slug {
                        return Err(slug_conflict(&candidate.slug));
                    }
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            revisions::record(
                &transaction,
                tenant,
                &entry,
                Reason::Initial,
                Some(principal.user_id()),
                entry.created_at,
            )
            .await?;
            let mut after = summary(&entry);
            if let (Some(source), Some(object)) = (source, after.as_object_mut()) {
                object.insert("translation_of".to_owned(), json!(source));
            }
            self.enqueue_index(tenant, &transaction, entry.id).await?;
            let event =
                events::EntryCreated(Change { before: None, after: Some(after), published: None });
            self.outbox
                .record(
                    &transaction,
                    tenant,
                    Actor::from(principal),
                    events::entry_subject(entry.id),
                    &event,
                )
                .await?;
            transaction.commit().await?;
            tracing::info!(entry_id = %entry.id, kind = %entry.kind, "content entry created");
            return Ok(entry);
        }
        Err(Error::conflict("content.slug_taken", "Could not find a free slug; give one yourself."))
    }

    /// Mục `id`. Cần `content.read`.
    pub(crate) async fn get(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Entry, Error> {
        principal.require(&CONTENT_READ)?;
        Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| entry_not_found(id))
    }

    /// Một trang mục theo bộ lọc. Cần `content.read`.
    pub(crate) async fn list(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        filter: &EntryFilter,
        page: &PageRequest,
    ) -> Result<Page<Entry>, Error> {
        principal.require(&CONTENT_READ)?;
        let mut select = Entity::find().scoped(tenant);
        if let Some(kind) = filter.kind {
            select = select.filter(Column::Kind.eq(kind.as_str()));
        }
        if let Some(locale) = &filter.locale {
            select = select.filter(Column::Locale.eq(locale.as_str()));
        }
        if let Some(status) = filter.status {
            select = select.filter(Column::Status.eq(status.as_str()));
        }
        select = match filter.parent {
            ParentFilter::Any => select,
            ParentFilter::Root => select.filter(Column::ParentKey.eq(ROOT)),
            ParentFilter::Id(id) => select.filter(Column::ParentKey.eq(id)),
        };
        if let Some(search) = filter.search.as_deref().filter(|search| !search.trim().is_empty()) {
            select = select.filter(autoxide_kernel::search::contains(Column::SearchKey, search));
        }
        if let Some(group) = filter.translation_group_id {
            select = select.filter(Column::TranslationGroupId.eq(group));
        }
        if let Some(term) = filter.term_id {
            select = select.filter(crate::taxonomy::terms::has_term(tenant, term));
        }
        if let Some(author) = filter.author_id {
            select = select.filter(crate::taxonomy::authors::has_author(tenant, author));
        }
        select = match filter.review_requested {
            Some(true) => select.filter(Column::ReviewRequestedAt.is_not_null()),
            Some(false) => select.filter(Column::ReviewRequestedAt.is_null()),
            None => select,
        };
        if let Some(scheduled) = filter.scheduled {
            let waiting = sea_orm::sea_query::Query::select()
                .column(crate::entity::schedule::Column::EntryId)
                .from(crate::entity::schedule::Entity)
                .and_where(crate::entity::schedule::Column::TenantId.eq(tenant.id().as_uuid()))
                .and_where(crate::entity::schedule::Column::Status.eq("scheduled"))
                .to_owned();
            select = if scheduled {
                select.filter(Column::Id.in_subquery(waiting))
            } else {
                select.filter(Column::Id.not_in_subquery(waiting))
            };
        }
        if let Some(creator) = filter.created_by {
            select = select.filter(Column::CreatedBy.eq(creator));
        }
        if let Some(ai) = filter.ai_generated {
            select = select.filter(Column::AiGenerated.eq(ai));
        }
        if let Some(missing) = &filter.missing_locale {
            // Nhóm bản dịch đã có mục bằng ngôn ngữ đó thì loại; mục chính nó bằng ngôn ngữ đó
            // cũng bị loại theo (cùng nhóm).
            let translated = sea_orm::sea_query::Query::select()
                .column(Column::TranslationGroupId)
                .from(Entity)
                .and_where(Column::TenantId.eq(tenant.id().as_uuid()))
                .and_where(Column::Locale.eq(missing.as_str()))
                .to_owned();
            select = select.filter(Column::TranslationGroupId.not_in_subquery(translated));
        }
        let read = tenant.database().read();
        match filter.sort {
            EntrySort::CreatedAt => fetch_page(select, Column::Id, |row| row.id, page, read).await,
            EntrySort::UpdatedAt => {
                let key = |row: &Entry| SortValue::Time(row.updated_at);
                fetch_page_by(select, Column::UpdatedAt, key, Column::Id, |row| row.id, page, read)
                    .await
            }
        }
    }

    /// Đường dẫn và lúc đăng của bản công khai của từng mục trong `ids` (một câu cho cả trang).
    pub(crate) async fn published_paths(
        &self,
        tenant: &TenantContext,
        ids: &[Uuid],
    ) -> Result<std::collections::HashMap<Uuid, (String, DateTime<Utc>)>, Error> {
        if ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        use crate::entity::published;
        let rows: Vec<(Uuid, String, DateTime<Utc>)> = published::Entity::find()
            .scoped(tenant)
            .select_only()
            .column(published::Column::EntryId)
            .column(published::Column::Path)
            .column(published::Column::PublishedAt)
            .filter(published::Column::EntryId.is_in(ids.iter().copied()))
            .into_tuple()
            .all(tenant.database().read())
            .await?;
        Ok(rows.into_iter().map(|(id, path, at)| (id, (path, at))).collect())
    }

    /// Sửa bản nháp của mục `id`. Cần `content.write`. Mục đang công khai thì bật
    /// `has_unpublished_changes`; bản công khai không đổi tới khi đăng lại.
    pub(crate) async fn update(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        fields: EntryFields,
        save_revision: bool,
        expected: Option<DateTime<Utc>>,
    ) -> Result<Entry, Error> {
        let revise =
            Revise { forced: save_revision.then_some(Reason::ExplicitSave), restored_from: None };
        self.update_inner(tenant, principal, id, fields, revise, expected).await
    }

    /// Sửa bản nháp; `revise`: revision bắt buộc ghi và có phải khôi phục không.
    pub(crate) async fn update_inner(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        fields: EntryFields,
        revise: Revise,
        expected: Option<DateTime<Utc>>,
    ) -> Result<Entry, Error> {
        let Revise { forced, restored_from } = revise;
        permissions::require_create(principal)?;
        let before = Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| entry_not_found(id))?;
        permissions::require_write(principal, &before)?;
        if expected.is_some_and(|expected| expected != before.updated_at) {
            return Err(autoxide_kernel::version::changed());
        }
        let mut problems = Vec::new();
        let mut candidate = before.clone();
        let slug_changed = fields.slug.as_ref().is_some_and(|slug| slug.trim() != before.slug);
        self.apply(tenant, &mut candidate, fields, &mut problems).await?;
        mark_agent_written(principal, &mut candidate);
        if !problems.is_empty() {
            return Err(invalid(problems));
        }
        let now = now();
        candidate.updated_at = now;
        if Status::of(&before) == Status::Published {
            candidate.has_unpublished_changes = true;
        }
        let reason = if forced.is_some() {
            forced
        } else if now - before.revised_at > chrono::Duration::minutes(BACKGROUND_SAVE_MINUTES) {
            Some(Reason::BackgroundSave)
        } else {
            None
        };
        if reason.is_some() {
            candidate.revised_at = now;
        }

        let transaction = tenant.database().begin_write().await?;
        if slug_changed && slug_taken(&transaction, tenant, &candidate).await? {
            return Err(slug_conflict(&candidate.slug));
        }
        let mut update =
            Entity::update(changed_columns(&before, &candidate)).validate()?.scoped(tenant);
        // Phiên bản đã đọc (`If-Match`), hay ít nhất "chưa ai sửa từ lúc đọc ở trên".
        update = update.filter(Column::UpdatedAt.eq(expected.unwrap_or(before.updated_at)));
        let after = match update.exec(&transaction).await {
            Ok(after) => after,
            Err(sea_orm::DbErr::RecordNotUpdated) => {
                return Err(autoxide_kernel::version::changed());
            }
            Err(error) if is_unique_violation(&error) => {
                return Err(slug_conflict(&candidate.slug));
            }
            Err(error) => return Err(error.into()),
        };
        if let Some(reason) = reason {
            revisions::record(&transaction, tenant, &after, reason, Some(principal.user_id()), now)
                .await?;
        }
        let mut after_summary = summary(&after);
        if let (Some(revision), Some(object)) = (restored_from, after_summary.as_object_mut()) {
            object.insert("restored_from".to_owned(), json!(revision));
        }
        let change =
            Change { before: Some(summary(&before)), after: Some(after_summary), published: None };
        self.enqueue_index(tenant, &transaction, id).await?;
        let actor = Actor::from(principal);
        let subject = events::entry_subject(id);
        if restored_from.is_some() {
            self.outbox
                .record(&transaction, tenant, actor, subject, &events::EntryRestored(change))
                .await?;
        } else {
            self.outbox
                .record(&transaction, tenant, actor, subject, &events::EntryUpdated(change))
                .await?;
        }
        transaction.commit().await?;
        Ok(after)
    }

    /// Áp `fields` lên `candidate` rồi kiểm toàn bộ (mọi lỗi vào `problems`); tính văn bản
    /// thuần, số chữ, thời gian đọc, khoá tìm; kiểm ảnh có tồn tại (qua `media`).
    pub(crate) async fn apply(
        &self,
        tenant: &TenantContext,
        candidate: &mut Entry,
        fields: EntryFields,
        problems: &mut Vec<FieldError>,
    ) -> Result<(), Error> {
        let trim = |value: Option<String>| value.map(|value| value.trim().to_owned());
        if let Some(title) = trim(fields.title) {
            candidate.title = title;
        }
        if (candidate.title.is_empty() || candidate.title.chars().count() > TITLE_MAX)
            && !problems.iter().any(|p| p.field == "title")
        {
            problems.push(
                FieldError::new("title", "length", "must be 1 to {title_max} characters")
                    .with_param("title_max", TITLE_MAX),
            );
        }
        match trim(fields.slug) {
            Some(slug) if slug.is_empty() => {
                candidate.slug = crate::entries::slug::from_title(&candidate.title, candidate.id);
            }
            Some(slug) => {
                if !crate::entries::slug::is_valid(&slug) {
                    problems.push(FieldError::new(
                        "slug",
                        "invalid",
                        "must be lower-case letters and digits in groups joined by `-`, at most 100 characters",
                    ));
                }
                candidate.slug = slug;
            }
            None if candidate.slug.is_empty() => {
                candidate.slug = crate::entries::slug::from_title(&candidate.title, candidate.id);
            }
            None => {}
        }
        for (field, value, target, max) in [
            ("excerpt", fields.excerpt, &mut candidate.excerpt, EXCERPT_MAX),
            (
                "featured_media_alt",
                fields.featured_media_alt,
                &mut candidate.featured_media_alt,
                ALT_MAX,
            ),
            ("meta_title", fields.meta_title, &mut candidate.meta_title, META_TITLE_MAX),
            (
                "meta_description",
                fields.meta_description,
                &mut candidate.meta_description,
                META_DESCRIPTION_MAX,
            ),
        ] {
            check_length(field, value.as_deref(), max, problems);
            if let Some(value) = value {
                *target = value.trim().to_owned();
            }
        }
        check_url("canonical_url", fields.canonical_url.as_deref(), problems);
        if let Some(url) = fields.canonical_url {
            candidate.canonical_url = url.trim().to_owned();
        }
        if let Some(noindex) = fields.noindex {
            candidate.noindex = noindex;
        }
        if let Some(ai_generated) = fields.ai_generated {
            candidate.ai_generated = ai_generated;
        }
        if let Some(revision) = fields.translated_from {
            candidate.translated_from_revision_id = Some(revision);
        }
        if let Some(featured) = fields.featured_media_id {
            candidate.featured_media_id = featured;
        }
        if let Some(og) = fields.og_media_id {
            candidate.og_media_id = og;
        }
        let body = match fields.body {
            Some(body) => body,
            None => serde_json::from_str(&candidate.body).unwrap_or_else(|_| body::empty()),
        };
        let info = match body::check(&body, self.config.max_body_bytes, &self.config.embed_hosts) {
            Ok(info) => info,
            Err(errors) => {
                problems.extend(errors);
                body::BodyInfo::default()
            }
        };
        candidate.body = serde_json::to_string(&body).map_err(Error::internal)?;
        candidate.body_schema_version = body::SCHEMA_VERSION;
        candidate.word_count = info.word_count;
        candidate.reading_time_minutes =
            body::reading_minutes(info.word_count, self.config.words_per_minute);
        candidate.search_key = autoxide_kernel::search::fold(&format!(
            "{}\n{}\n{}",
            candidate.title, candidate.excerpt, info.plain_text
        ));
        candidate.plain_text = info.plain_text;
        self.check_media(tenant, candidate, &info.media_ids, false, problems).await
    }

    /// Ảnh mục dùng (ảnh đại diện, ảnh chia sẻ, ảnh trong bài) có tồn tại — và, khi
    /// `public`, có công khai — không. Gọi `media` ngoài transaction.
    pub(crate) async fn check_media(
        &self,
        tenant: &TenantContext,
        entry: &Entry,
        in_body: &BTreeSet<Uuid>,
        public: bool,
        problems: &mut Vec<FieldError>,
    ) -> Result<(), Error> {
        let mut wanted: Vec<(String, Uuid)> = Vec::new();
        if let Some(id) = entry.featured_media_id {
            wanted.push(("featured_media_id".to_owned(), id));
        }
        if let Some(id) = entry.og_media_id {
            wanted.push(("og_media_id".to_owned(), id));
        }
        wanted.extend(in_body.iter().map(|id| ("body".to_owned(), *id)));
        if wanted.is_empty() {
            return Ok(());
        }
        let ids: Vec<Uuid> =
            wanted.iter().map(|(_, id)| *id).collect::<BTreeSet<_>>().into_iter().collect();
        let mut found = std::collections::HashMap::new();
        for chunk in ids.chunks(autoxide_media::api::MAX_IDS) {
            for item in self.media.find_many(tenant, chunk).await? {
                found.insert(item.id, item);
            }
        }
        for (field, id) in wanted {
            match found.get(&id) {
                None => problems.push(
                    FieldError::new(field, "media_not_found", "media item {id} does not exist")
                        .with_param("id", id),
                ),
                Some(item)
                    if public
                        && item.visibility != autoxide_media::api::MediaVisibility::Public =>
                {
                    problems.push(
                        FieldError::new(field, "media_not_public", "media item {id} is not public")
                            .with_param("id", id),
                    );
                }
                Some(_) => {}
            }
        }
        Ok(())
    }

    /// Kiểm trang cha `parent` cho `entry`: là trang, cùng ngôn ngữ, cây không quá sâu.
    pub(crate) async fn check_parent<C: ConnectionTrait>(
        &self,
        tenant: &TenantContext,
        connection: &C,
        entry: &Entry,
        parent: Uuid,
        problems: &mut Vec<FieldError>,
    ) -> Result<(), Error> {
        let Some(row) = Entity::find_by_id(parent).scoped(tenant).one(connection).await? else {
            problems.push(FieldError::new("parent_id", "not_found", "no page has this id"));
            return Ok(());
        };
        if row.kind != Kind::Page.as_str() || row.locale != entry.locale {
            problems.push(FieldError::new(
                "parent_id",
                "invalid",
                "must be a page in the same language",
            ));
            return Ok(());
        }
        let below = subtree_height(connection, tenant, entry.id).await?;
        if depth(connection, tenant, parent).await? + below > MAX_TREE_DEPTH {
            problems.push(
                FieldError::new(
                    "parent_id",
                    "tree_too_deep",
                    "pages may nest at most {max_tree_depth} levels",
                )
                .with_param("max_tree_depth", MAX_TREE_DEPTH),
            );
        }
        Ok(())
    }

    /// Tạo bản dịch của mục `source_id` sang ngôn ngữ `locale`: một mục mới (nháp) cùng nhóm
    /// bản dịch, chép nội dung từ bản gốc rồi áp `fields`. Bản dịch của trang có cha là bản
    /// dịch của trang cha trong ngôn ngữ đó. Cần `content.write`, hoặc `content.write_own` khi
    /// bản gốc là của mình.
    pub(crate) async fn create_translation(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        source_id: Uuid,
        locale: &str,
        fields: EntryFields,
    ) -> Result<Entry, Error> {
        permissions::require_create(principal)?;
        let source = Entity::find_by_id(source_id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| entry_not_found(source_id))?;
        permissions::require_write(principal, &source)?;
        let mut problems = Vec::new();
        let target = self.check_locale(Some(locale), "locale", &mut problems);
        if problems.is_empty() && target == source.locale {
            problems.push(FieldError::new(
                "locale",
                "invalid",
                "must differ from the language of the entry",
            ));
        }
        if !problems.is_empty() {
            return Err(invalid(problems));
        }
        let sibling = Entity::find()
            .scoped(tenant)
            .filter(Column::TranslationGroupId.eq(source.translation_group_id))
            .filter(Column::Locale.eq(target.as_str()))
            .one(tenant.database().read())
            .await?;
        if sibling.is_some() {
            return Err(Error::conflict(
                "content.translation_exists",
                "This entry already has a translation in `{locale}`.",
            )
            .with_param("locale", &target));
        }
        let parent = match source.parent_id {
            None => None,
            Some(parent) => {
                let parent = Entity::find_by_id(parent)
                    .scoped(tenant)
                    .one(tenant.database().read())
                    .await?
                    .ok_or_else(|| entry_not_found(parent))?;
                let translated = Entity::find()
                    .scoped(tenant)
                    .filter(Column::TranslationGroupId.eq(parent.translation_group_id))
                    .filter(Column::Locale.eq(target.as_str()))
                    .one(tenant.database().read())
                    .await?;
                match translated {
                    Some(translated) => {
                        // Bản dịch nằm dưới bản dịch của trang cha: cùng luật "trang cha phải là
                        // của mình" như khi tạo trang con.
                        permissions::require_parent(principal, &translated)?;
                        Some(translated.id)
                    }
                    None => {
                        return Err(Error::conflict(
                            "content.parent_not_translated",
                            "The parent page has no `{locale}` translation yet. Translate it first.",
                        )
                        .with_param("locale", &target));
                    }
                }
            }
        };
        let now = now();
        let mut candidate = source.clone();
        candidate.id = Uuid::now_v7();
        candidate.locale = target;
        candidate.parent_id = parent;
        candidate.parent_key = parent.unwrap_or(ROOT);
        candidate.status = Status::Draft.as_str().to_owned();
        candidate.has_unpublished_changes = false;
        candidate.created_by = Some(principal.user_id());
        candidate.created_by_api_key_id = principal.api_key_id();
        // Bản dịch là mục mới: không mang cờ của bản gốc; ai tạo bằng AI thì tự khai (hay máy
        // chủ đặt khi nó dịch).
        candidate.ai_generated = false;
        candidate.review_requested_at = None;
        candidate.review_requested_by = None;
        candidate.translated_from_revision_id = None;
        candidate.created_at = now;
        candidate.updated_at = now;
        candidate.revised_at = now;
        // Không gửi slug: lấy slug của bản gốc; gửi `""`: sinh từ tiêu đề (đã dịch). Cả hai tự
        // thêm `-2`, `-3`… khi trùng.
        let auto_slug = fields.slug.as_deref().is_none_or(|slug| slug.trim().is_empty());
        self.apply(tenant, &mut candidate, fields, &mut problems).await?;
        mark_agent_written(principal, &mut candidate);
        if !problems.is_empty() {
            return Err(invalid(problems));
        }
        self.insert(tenant, principal, candidate, auto_slug, Some(source.id)).await
    }

    async fn translation_taken(
        &self,
        tenant: &TenantContext,
        candidate: &Entry,
    ) -> Result<bool, Error> {
        Ok(Entity::find()
            .scoped(tenant)
            .filter(Column::TranslationGroupId.eq(candidate.translation_group_id))
            .filter(Column::Locale.eq(candidate.locale.as_str()))
            .one(tenant.database().write())
            .await?
            .is_some())
    }
}

/// Người chỉ có `content.write_own` (thường là AI agent) không tự khai được `ai_generated`: máy
/// chủ luôn đặt `true`, để người đọc không bị giấu nội dung do máy viết (D-109).
fn mark_agent_written(principal: &Principal, candidate: &mut Entry) {
    if permissions::write_own_only(principal) {
        candidate.ai_generated = true;
    }
}

/// Một mục rỗng với các giá trị mặc định.
pub(crate) fn blank(
    tenant: &TenantContext,
    id: Uuid,
    kind: Kind,
    locale: &str,
    parent: Option<Uuid>,
    by: Uuid,
    now: DateTime<Utc>,
) -> Entry {
    Entry {
        id,
        tenant_id: tenant.id().as_uuid(),
        kind: kind.as_str().to_owned(),
        locale: locale.to_owned(),
        translation_group_id: id,
        parent_id: parent,
        parent_key: parent.unwrap_or(ROOT),
        status: Status::Draft.as_str().to_owned(),
        has_unpublished_changes: false,
        title: String::new(),
        slug: String::new(),
        excerpt: String::new(),
        body: body::empty().to_string(),
        body_schema_version: body::SCHEMA_VERSION,
        plain_text: String::new(),
        search_key: String::new(),
        word_count: 0,
        reading_time_minutes: 0,
        featured_media_id: None,
        featured_media_alt: String::new(),
        meta_title: String::new(),
        meta_description: String::new(),
        og_media_id: None,
        canonical_url: String::new(),
        noindex: false,
        created_by: Some(by),
        created_by_api_key_id: None,
        ai_generated: false,
        review_requested_at: None,
        review_requested_by: None,
        translated_from_revision_id: None,
        created_at: now,
        updated_at: now,
        revised_at: now,
    }
}

/// `ActiveModel` chỉ đánh dấu các cột khác nhau giữa `before` và `after`.
pub(crate) fn changed_columns(before: &Entry, after: &Entry) -> entry::ActiveModel {
    let mut row: entry::ActiveModel = before.clone().into_active_model();
    macro_rules! set_if_changed {
        ($($field:ident),*) => {
            $(if before.$field != after.$field { row.$field = Set(after.$field.clone()); })*
        };
    }
    set_if_changed!(
        parent_id,
        parent_key,
        status,
        has_unpublished_changes,
        title,
        slug,
        excerpt,
        body,
        body_schema_version,
        plain_text,
        search_key,
        word_count,
        reading_time_minutes,
        featured_media_id,
        featured_media_alt,
        meta_title,
        meta_description,
        og_media_id,
        canonical_url,
        noindex,
        created_by,
        created_by_api_key_id,
        ai_generated,
        review_requested_at,
        review_requested_by,
        translated_from_revision_id,
        updated_at,
        revised_at,
        translation_group_id
    );
    row
}

/// Những gì audit cần biết về một mục (thân bài không chép vào audit — quá lớn — chỉ số chữ).
pub(crate) fn summary(entry: &Entry) -> Value {
    json!({
        "kind": entry.kind,
        "locale": entry.locale,
        "status": entry.status,
        "title": entry.title,
        "slug": entry.slug,
        "parent_id": entry.parent_id,
        "excerpt": entry.excerpt,
        "word_count": entry.word_count,
        "featured_media_id": entry.featured_media_id,
        "meta_title": entry.meta_title,
        "meta_description": entry.meta_description,
        "noindex": entry.noindex,
        "ai_generated": entry.ai_generated,
    })
}

/// Khoá dòng mục `id` tới hết transaction (`rows::lock_one`, D-181). `false`: không còn.
pub(crate) async fn lock<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    id: Uuid,
) -> Result<bool, Error> {
    Ok(rows::lock_one(connection, Entity::find_by_id(id).scoped(tenant)).await?.is_some())
}

/// Khoá dòng mục `id` tới hết `transaction` (như [`lock`]) rồi đọc nó trong transaction đó;
/// mục không còn: `content.not_found`.
pub(crate) async fn lock_entry<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    id: Uuid,
) -> Result<Entry, Error> {
    // Khoá và đọc trong một câu.
    rows::lock_one(transaction, Entity::find_by_id(id).scoped(tenant))
        .await?
        .ok_or_else(|| entry_not_found(id))
}

/// Cấp của trang `id` trong cây (trang gốc: 1).
pub(crate) async fn depth<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    id: Uuid,
) -> Result<usize, Error> {
    let mut current = Some(id);
    let mut level = 0;
    while let Some(id) = current {
        level += 1;
        if level > 100 {
            return Err(Error::internal(
                "the page tree is deeper than expected; it may contain a cycle",
            ));
        }
        current = Entity::find_by_id(id)
            .scoped(tenant)
            .one(connection)
            .await?
            .and_then(|row| row.parent_id);
    }
    Ok(level)
}

/// Số cấp của cây con bắt đầu từ mục `id` (chỉ có nó: 1).
pub(crate) async fn subtree_height<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    id: Uuid,
) -> Result<usize, Error> {
    let mut level = 1;
    let mut current = vec![id];
    loop {
        let children: Vec<Uuid> = Entity::find()
            .scoped(tenant)
            .select_only()
            .column(Column::Id)
            .filter(Column::ParentKey.is_in(current))
            .into_tuple()
            .all(connection)
            .await?;
        if children.is_empty() || level > MAX_TREE_DEPTH + 1 {
            return Ok(level);
        }
        level += 1;
        current = children;
    }
}

/// Slug của `candidate` đã có mục khác dùng trong cùng phạm vi (ngôn ngữ, loại, cha).
async fn slug_taken<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    candidate: &Entry,
) -> Result<bool, Error> {
    Ok(Entity::find()
        .scoped(tenant)
        .filter(Column::Locale.eq(candidate.locale.as_str()))
        .filter(Column::Kind.eq(candidate.kind.as_str()))
        .filter(Column::ParentKey.eq(candidate.parent_key))
        .filter(Column::Slug.eq(candidate.slug.as_str()))
        .filter(Column::Id.ne(candidate.id))
        .one(connection)
        .await?
        .is_some())
}

/// Slug còn trống trong phạm vi của `candidate`: `base`, rồi `base-2`, `base-3`…
async fn free_slug<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    candidate: &Entry,
    base: &str,
) -> Result<String, Error> {
    let taken: BTreeSet<String> = Entity::find()
        .scoped(tenant)
        .select_only()
        .column(Column::Slug)
        .filter(Column::Locale.eq(candidate.locale.as_str()))
        .filter(Column::Kind.eq(candidate.kind.as_str()))
        .filter(Column::ParentKey.eq(candidate.parent_key))
        .filter(Column::Id.ne(candidate.id))
        .filter(Column::Slug.eq(base).or(Column::Slug.starts_with(format!("{base}-"))))
        .into_tuple::<String>()
        .all(connection)
        .await?
        .into_iter()
        .collect();
    if !taken.contains(base) {
        return Ok(base.to_owned());
    }
    let mut n = 2;
    loop {
        let next = crate::entries::slug::with_suffix(base, n);
        if !taken.contains(&next) {
            return Ok(next);
        }
        n += 1;
    }
}

pub(crate) fn is_unique_violation(error: &sea_orm::DbErr) -> bool {
    matches!(error.sql_err(), Some(SqlErr::UniqueConstraintViolation(_)))
}

fn slug_conflict(slug: &str) -> Error {
    Error::conflict(
        "content.slug_taken",
        "The slug `{slug}` is already used by another entry in the same language and place. Choose another.",
    )
    .with_param("slug", slug)
}
