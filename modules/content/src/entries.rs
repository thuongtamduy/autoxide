//! Bản nháp của bài viết và trang: tạo, đọc, liệt kê, sửa. Mỗi bản dịch là một mục riêng
//! (D-102). Đăng, gỡ, chuyển, xoá ở `publishing.rs`.

pub(crate) mod body;
pub(crate) mod review;
pub(crate) mod revisions;

use autoxide_kernel::time::Timestamp;
use std::collections::BTreeSet;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, SortValue, fetch_page, fetch_page_by};
use autoxide_kernel::rows;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
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

mod checks;
mod create;
mod update;

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
    /// Viết hay sửa từ gợi ý của AI (Luật 134/2025/QH15, D-362). Bỏ (`false` khi đang `true`) cần
    /// `content.publish`.
    pub ai_assisted: Option<bool>,
    /// Chỉ máy chủ đặt (khi tự dịch): revision của bản gốc đã dịch. Không có trong API.
    pub translated_from: Option<Uuid>,
    /// Giá trị trường tự định nghĩa gửi lên (bước 51 P19); `null` trong map xoá một trường.
    pub custom_fields: Option<serde_json::Map<String, Value>>,
}

/// Một mục mới.
#[derive(Debug, Clone)]
pub(crate) struct NewEntry {
    pub kind: Kind,
    /// `None`: ngôn ngữ mặc định.
    pub locale: Option<String>,
    /// Trang cha (chỉ trang).
    pub parent_id: Option<Uuid>,
    /// Nút sở hữu (bước 34); `None`: nút duy nhất của người tạo (gốc khi có quyền cả khách).
    pub org_id: Option<Uuid>,
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
    /// Chỉ mục của nút này (bộ lọc quản trị `owner_org`, bước 34).
    pub owner_org: Option<Uuid>,
    /// Chỉ mục hiện trên website của nút này: nút đó và các nút cấp trên (bộ lọc quản trị `site`).
    pub site: Option<Uuid>,
    /// Nhãn nội bộ (bước 51 P20): có một trong / có mọi nhãn này.
    pub tags_any: Vec<Uuid>,
    pub tags_all: Vec<Uuid>,
    /// Trường tự định nghĩa (bước 51 P19): (khoá, `<toán tử>:<giá trị>`) từ `cf.<khoá>=`.
    pub custom: Vec<(String, String)>,
}

/// Bỏ nhãn `ai_assisted` mà không có `content.publish`.
fn ai_label_locked() -> Error {
    Error::forbidden(
        "content.ai_label_requires_publish",
        "Only someone with permission `content.publish` may remove the `ai_assisted` label.",
    )
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
    now: Timestamp,
) -> Entry {
    Entry {
        id,
        tenant_id: tenant.id().as_uuid(),
        org_id: Some(autoxide_kernel::tenancy::root_of(tenant.id()).as_uuid()),
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
        ai_assisted: false,
        restricted_at: None,
        custom_fields: None,
        review_requested_at: None,
        review_requested_by: None,
        translated_from_revision_id: None,
        created_at: now,
        updated_at: now,
        revised_at: now,
    }
}

/// `ActiveModel` chỉ đánh dấu các cột khác nhau giữa `before` và `after`.
/// Giá trị trường tự định nghĩa đã lưu trên bài.
pub(crate) fn stored_custom(entry: &Entry) -> Option<serde_json::Map<String, Value>> {
    entry.custom_fields.as_deref().and_then(|text| serde_json::from_str(text).ok())
}

pub(crate) fn changed_columns(before: &Entry, after: &Entry) -> entry::ActiveModel {
    let mut row: entry::ActiveModel = before.clone().into_active_model();
    macro_rules! set_if_changed {
        ($($field:ident),*) => {
            $(if before.$field != after.$field { row.$field = Set(after.$field.clone()); })*
        };
    }
    set_if_changed!(
        custom_fields,
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
        ai_assisted,
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
        "ai_assisted": entry.ai_assisted,
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

/// Cùng nút (website) với `candidate`: mỗi website một bộ slug, một bộ đường dẫn (D-307).
fn same_site(candidate: &Entry) -> sea_orm::Condition {
    match candidate.org_id {
        Some(org) => sea_orm::Condition::all().add(Column::OrgId.eq(org)),
        None => sea_orm::Condition::all().add(Column::OrgId.is_null()),
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
        .filter(same_site(candidate))
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
        .filter(same_site(candidate))
        .filter(Column::Id.ne(candidate.id))
        .filter(
            Column::Slug
                .eq(base)
                .or(autoxide_kernel::search::starts_with(Column::Slug, &format!("{base}-"))),
        )
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
        let next = autoxide_kernel::slug::with_suffix(base, n);
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
