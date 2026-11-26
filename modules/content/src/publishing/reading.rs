//! Đọc bản công khai cho người đọc (không cần đăng nhập): chỉ từ `content_published`, nên
//! không bao giờ lộ bản nháp (D-102). Ghép thêm ảnh (qua `media`: ảnh không còn công khai coi
//! như không có), các bản dịch đang công khai (cho `hreflang`), danh mục, thẻ, tác giả.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use autoxide_kernel::error::Error;
use autoxide_media::api::{MediaRef, MediaVisibility};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use uuid::Uuid;

use crate::entity::published;
use crate::publishing::Published;
use crate::service::ContentService;
use crate::taxonomy::authors::AuthorRef;
use crate::taxonomy::terms::TermRef;

/// Một mục công khai kèm những gì website cần để hiển thị.
#[derive(Debug, Clone)]
pub(crate) struct PublicView {
    pub row: Published,
    pub featured: Option<MediaRef>,
    pub og: Option<MediaRef>,
    /// Ảnh trong thân bài (node `image`) còn công khai, theo thứ tự xuất hiện.
    pub body_media: Vec<MediaRef>,
    /// (ngôn ngữ, đường dẫn) của các bản dịch khác đang công khai.
    pub translations: Vec<(String, String)>,
    pub terms: Vec<TermRef>,
    pub authors: Vec<AuthorRef>,
    /// Bản ghi nhúng (`<kind>:<id>` → đối tượng) còn công khai, theo ngôn ngữ của bài (bước 51 P6).
    pub embeds: BTreeMap<String, autoxide_kernel::resource::Resource>,
    /// Trường tự định nghĩa mức `public` (bước 51 P19).
    pub custom_fields: serde_json::Map<String, serde_json::Value>,
}

impl ContentService {
    /// Ghép thông tin cho các bản công khai `rows`, giữ thứ tự. Số câu query không đổi theo số
    /// mục (không N+1).
    pub(crate) async fn public_views(
        &self,
        tenant: &TenantContext,
        rows: Vec<Published>,
    ) -> Result<Vec<PublicView>, Error> {
        let ids: Vec<Uuid> = rows.iter().map(|row| row.entry_id).collect();
        let default_locale = self.languages(tenant).await?.default;
        // Một câu đọc định nghĩa cho cả trang; không có trường công khai thì không giải JSON.
        let public_keys = if rows.iter().any(|row| row.custom_fields.is_some()) {
            self.custom_fields.public_keys(tenant, crate::resource::ENTRY).await?
        } else {
            BTreeSet::new()
        };
        // Mục của loại tự định nghĩa (D-235): trường công khai của chính loại, một lần mỗi loại.
        let mut type_keys: std::collections::HashMap<Uuid, BTreeSet<String>> = Default::default();
        let typed: BTreeSet<Uuid> = rows
            .iter()
            .filter(|row| row.custom_fields.is_some())
            .filter_map(|row| row.type_id)
            .collect();
        if !typed.is_empty() {
            let types = crate::entity::content_type::Entity::find()
                .scoped(tenant)
                .filter(crate::entity::content_type::Column::Id.is_in(typed))
                .all(tenant.database().read())
                .await?;
            for row in types {
                let target = crate::types::field_target(&row.key);
                type_keys.insert(row.id, self.custom_fields.public_keys(tenant, &target).await?);
            }
        }
        let mut authors =
            crate::taxonomy::authors::authors_of(tenant.database().read(), tenant, &ids).await?;
        // Ảnh đại diện của tác giả cũng tra qua `media`: ảnh riêng tư thì không lộ cả ID.
        let avatars: Vec<Uuid> =
            authors.values().flatten().filter_map(|author| author.avatar_media_id).collect();
        // Ảnh trong thân bài: website cần kích thước và các bản nhỏ của chúng cho `srcset`
        // (D-168). Ảnh tra theo ngôn ngữ của bài, để `alt` là bản dịch của ngôn ngữ đó nếu có
        // (D-202); ảnh đại diện tra chung với ngôn ngữ mặc định.
        let body_images: Vec<Vec<Uuid>> =
            rows.iter().map(|row| self.body_media_ids(&row.body)).collect();
        let mut wanted: BTreeMap<String, BTreeSet<Uuid>> = BTreeMap::new();
        for (row, images) in rows.iter().zip(&body_images) {
            let ids = wanted.entry(row.locale.clone()).or_default();
            ids.extend([row.featured_media_id, row.og_media_id].into_iter().flatten());
            ids.extend(images.iter().copied());
        }
        if !avatars.is_empty() {
            wanted.entry(default_locale.clone()).or_default().extend(avatars);
        }
        let mut media: HashMap<(String, Uuid), MediaRef> = HashMap::new();
        let mut public: HashSet<Uuid> = HashSet::new();
        for (locale, ids) in wanted {
            let ids: Vec<Uuid> = ids.into_iter().collect();
            for chunk in ids.chunks(autoxide_media::api::MAX_IDS) {
                for item in self.media.find_many_in(tenant, chunk, &locale).await? {
                    if item.visibility == MediaVisibility::Public {
                        public.insert(item.id);
                        media.insert((locale.clone(), item.id), item);
                    }
                }
            }
        }
        let groups: Vec<Uuid> = rows
            .iter()
            .map(|row| row.translation_group_id)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let siblings = if groups.is_empty() {
            Vec::new()
        } else {
            published::Entity::find()
                .scoped(tenant)
                .filter(published::Column::TranslationGroupId.is_in(groups))
                .filter(published::Column::RestrictedAt.is_null())
                .all(tenant.database().read())
                .await?
        };
        let mut terms = crate::taxonomy::terms::terms_of(
            tenant.database().read(),
            tenant,
            &ids,
            &default_locale,
            &rows,
        )
        .await?;
        // Bản ghi nhúng: mỗi (ngôn ngữ, loại) một lần tra; chỉ giữ đối tượng còn công khai.
        let body_records: Vec<Vec<(String, String)>> =
            rows.iter().map(|row| crate::entries::body::stored_records(&row.body)).collect();
        let mut wanted_records: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
        for (row, records) in rows.iter().zip(&body_records) {
            for (kind, id) in records {
                wanted_records
                    .entry((row.locale.clone(), kind.clone()))
                    .or_default()
                    .insert(id.clone());
            }
        }
        let mut objects: HashMap<(String, String, String), autoxide_kernel::resource::Resource> =
            HashMap::new();
        for ((locale, kind), ids) in wanted_records {
            if !self.resources.knows(&kind) {
                continue;
            }
            let ids: Vec<String> = ids.into_iter().collect();
            let found = self.resources.resolve_many(tenant, &kind, &ids, Some(&locale)).await?;
            for (id, resource) in found.into_iter().filter(|(_, resource)| resource.public) {
                objects.insert((locale.clone(), kind.clone(), id), resource);
            }
        }
        for author in authors.values_mut().flatten() {
            if author.avatar_media_id.is_some_and(|id| !public.contains(&id)) {
                author.avatar_media_id = None;
            }
        }
        Ok(rows
            .into_iter()
            .zip(body_images)
            .zip(body_records)
            .map(|((row, images), records)| {
                let embeds = records
                    .into_iter()
                    .filter_map(|(kind, id)| {
                        let resource =
                            objects.get(&(row.locale.clone(), kind.clone(), id.clone()))?.clone();
                        Some((format!("{kind}:{id}"), resource))
                    })
                    .collect();
                let translations = siblings
                    .iter()
                    .filter(|other| {
                        other.translation_group_id == row.translation_group_id
                            && other.entry_id != row.entry_id
                    })
                    .map(|other| (other.locale.clone(), other.path.clone()))
                    .collect();
                let image = |id: Uuid| media.get(&(row.locale.clone(), id)).cloned();
                PublicView {
                    custom_fields: autoxide_kernel::custom_fields::CustomFields::pick_public(
                        row.custom_fields.as_deref(),
                        row.type_id.and_then(|id| type_keys.get(&id)).unwrap_or(&public_keys),
                    ),
                    featured: row.featured_media_id.and_then(image),
                    og: row.og_media_id.and_then(image),
                    body_media: images.iter().filter_map(|id| image(*id)).collect(),
                    translations,
                    terms: terms.remove(&row.entry_id).unwrap_or_default(),
                    authors: authors.remove(&row.entry_id).unwrap_or_default(),
                    embeds,
                    row,
                }
            })
            .collect())
    }
}

use autoxide_kernel::pagination::{Page, PageRequest, SortValue, fetch_page, fetch_page_by};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};

/// Bộ lọc danh sách bài công khai.
#[derive(Debug, Clone, Default)]
pub(crate) struct PostFilter {
    pub locale: String,
    /// Website (bước 34): nút của website và các nút cấp trên — chỉ bài của các nút này.
    pub site: Vec<Uuid>,
    pub term_id: Option<Uuid>,
    pub author_id: Option<Uuid>,
    pub search: Option<String>,
}

/// Bản công khai hiện trên website có chuỗi nút `lineage` (gần nhất trước, D-307): của một nút
/// trên chuỗi, và không bị một bài cùng ngôn ngữ, cùng đường dẫn ở nút GẦN website hơn che (hai
/// nút cùng có `/about` thì website chỉ thấy một). Một điều kiện SQL, không đổ dòng ra bộ nhớ.
pub(crate) fn on_site(lineage: &[Uuid]) -> sea_orm::Condition {
    use sea_orm::sea_query::{Alias, Expr, ExprTrait, Query};
    let mut any = sea_orm::Condition::any();
    for (at, org) in lineage.iter().enumerate() {
        let mut here = sea_orm::Condition::all().add(published::Column::OrgId.eq(*org));
        let nearer = &lineage[..at];
        if !nearer.is_empty() {
            let shadow = Alias::new("shadow");
            let same = |column: published::Column| {
                Expr::col((shadow.clone(), column)).equals((published::Entity, column))
            };
            let mut covered = Query::select();
            covered
                .expr(Expr::val(1))
                .from_as(published::Entity, shadow.clone())
                .and_where(same(published::Column::TenantId))
                .and_where(same(published::Column::Locale))
                .and_where(same(published::Column::Path))
                .and_where(
                    Expr::col((shadow.clone(), published::Column::OrgId))
                        .is_in(nearer.iter().copied()),
                );
            here = here.add(Expr::exists(covered).not());
        }
        any = any.add(here);
    }
    any
}

impl ContentService {
    /// Bài đã đăng của một ngôn ngữ, mới nhất trước (theo ngày đăng hiển thị).
    pub(crate) async fn public_posts(
        &self,
        tenant: &TenantContext,
        filter: &PostFilter,
        page: &PageRequest,
    ) -> Result<Page<Published>, Error> {
        let mut select = published::Entity::find()
            .scoped(tenant)
            .filter(published::Column::RestrictedAt.is_null())
            .filter(published::Column::Kind.eq("post"))
            .filter(published::Column::Locale.eq(filter.locale.as_str()))
            .filter(on_site(&filter.site));
        if let Some(term) = filter.term_id {
            select = select.filter(crate::taxonomy::terms::published_has_term(tenant, term));
        }
        if let Some(author) = filter.author_id {
            select = select.filter(crate::taxonomy::authors::published_has_author(tenant, author));
        }
        if let Some(search) = filter.search.as_deref().filter(|search| !search.trim().is_empty()) {
            select = select
                .filter(autoxide_kernel::search::contains(published::Column::SearchKey, search));
        }
        let key = |row: &Published| SortValue::Time(row.published_at);
        let read = tenant.database().read();
        fetch_page_by(
            select,
            published::Column::PublishedAt,
            key,
            published::Column::EntryId,
            |row| row.entry_id,
            page,
            read,
        )
        .await
    }

    /// Mọi mục công khai (một ngôn ngữ, hay mọi ngôn ngữ), cho sitemap; theo ID.
    pub(crate) async fn sitemap(
        &self,
        tenant: &TenantContext,
        site: &autoxide_kernel::tenancy::Site,
        locales: Option<&[String]>,
        page: &PageRequest,
    ) -> Result<Page<Published>, Error> {
        let mut select = published::Entity::find()
            .scoped(tenant)
            .filter(published::Column::RestrictedAt.is_null())
            .filter(on_site(&site.lineage_ids()))
            // Mục của loại không có trang (đường dẫn nội bộ `~…`) hay loại đã tắt không vào sitemap.
            .filter(published::Column::Path.starts_with("/"))
            .filter(crate::types::public::type_is_active(tenant));
        // Sandbox (D-232): sitemap rỗng — bản thử không được máy tìm kiếm thu thập.
        if tenant.is_sandbox() {
            select = select.filter(published::Column::EntryId.is_null());
        }
        if let Some(locales) = locales {
            select = select.filter(published::Column::Locale.is_in(locales.iter().cloned()));
        }
        fetch_page(
            select,
            published::Column::EntryId,
            |row| row.entry_id,
            page,
            tenant.database().read(),
        )
        .await
    }

    /// Một trang bài của `locale`, mới đăng trước (như `public_posts`), cho feed.
    pub(crate) async fn feed(
        &self,
        tenant: &TenantContext,
        site: &autoxide_kernel::tenancy::Site,
        locale: &str,
        page: &PageRequest,
    ) -> Result<Page<Published>, Error> {
        let filter = PostFilter {
            locale: locale.to_owned(),
            site: site.lineage_ids(),
            ..Default::default()
        };
        self.public_posts(tenant, &filter, page).await
    }

    /// Các bản dịch đang công khai của từng mục (ngôn ngữ, đường dẫn), cho sitemap.
    pub(crate) async fn translations_of(
        &self,
        tenant: &TenantContext,
        rows: &[Published],
    ) -> Result<HashMap<Uuid, Vec<(String, String)>>, Error> {
        let groups: Vec<Uuid> = rows
            .iter()
            .map(|row| row.translation_group_id)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if groups.is_empty() {
            return Ok(HashMap::new());
        }
        let siblings = published::Entity::find()
            .scoped(tenant)
            .filter(published::Column::TranslationGroupId.is_in(groups))
            .filter(published::Column::RestrictedAt.is_null())
            .all(tenant.database().read())
            .await?;
        Ok(rows
            .iter()
            .map(|row| {
                let others = siblings
                    .iter()
                    .filter(|other| {
                        other.translation_group_id == row.translation_group_id
                            && other.entry_id != row.entry_id
                    })
                    .map(|other| (other.locale.clone(), other.path.clone()))
                    .collect();
                (row.entry_id, others)
            })
            .collect())
    }
}
