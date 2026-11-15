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
}

impl ContentService {
    /// Ghép thông tin cho các bản công khai `rows`, giữ thứ tự. Số câu query không đổi theo số
    /// mục (không N+1).
    pub(crate) async fn public_views(
        &self,
        rows: Vec<Published>,
    ) -> Result<Vec<PublicView>, Error> {
        let ids: Vec<Uuid> = rows.iter().map(|row| row.entry_id).collect();
        let mut authors = crate::taxonomy::authors::authors_of(self.database.read(), &ids).await?;
        // Ảnh đại diện của tác giả cũng tra qua `media`: ảnh riêng tư thì không lộ cả ID.
        let avatars: Vec<Uuid> =
            authors.values().flatten().filter_map(|author| author.avatar_media_id).collect();
        // Ảnh trong thân bài: website cần kích thước và các bản nhỏ của chúng cho `srcset`
        // (D-168). Ảnh tra theo ngôn ngữ của bài, để `alt` là bản dịch của ngôn ngữ đó nếu có
        // (D-202); ảnh đại diện tra chung với ngôn ngữ mặc định.
        let body_images: Vec<Vec<Uuid>> =
            rows.iter().map(|row| crate::entries::body::stored_image_ids(&row.body)).collect();
        let mut wanted: BTreeMap<String, BTreeSet<Uuid>> = BTreeMap::new();
        for (row, images) in rows.iter().zip(&body_images) {
            let ids = wanted.entry(row.locale.clone()).or_default();
            ids.extend([row.featured_media_id, row.og_media_id].into_iter().flatten());
            ids.extend(images.iter().copied());
        }
        if !avatars.is_empty() {
            wanted.entry(self.config.default_locale.clone()).or_default().extend(avatars);
        }
        let mut media: HashMap<(String, Uuid), MediaRef> = HashMap::new();
        let mut public: HashSet<Uuid> = HashSet::new();
        for (locale, ids) in wanted {
            let ids: Vec<Uuid> = ids.into_iter().collect();
            for chunk in ids.chunks(autoxide_media::api::MAX_IDS) {
                for item in self
                    .media
                    .find_many_in(&TenantContext::legacy_default(&self.database), chunk, &locale)
                    .await?
                {
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
                .filter(published::Column::TranslationGroupId.is_in(groups))
                .all(self.database.read())
                .await?
        };
        let mut terms = crate::taxonomy::terms::terms_of(
            self.database.read(),
            &ids,
            &self.config.default_locale,
            &rows,
        )
        .await?;
        for author in authors.values_mut().flatten() {
            if author.avatar_media_id.is_some_and(|id| !public.contains(&id)) {
                author.avatar_media_id = None;
            }
        }
        Ok(rows
            .into_iter()
            .zip(body_images)
            .map(|(row, images)| {
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
                    featured: row.featured_media_id.and_then(image),
                    og: row.og_media_id.and_then(image),
                    body_media: images.iter().filter_map(|id| image(*id)).collect(),
                    translations,
                    terms: terms.remove(&row.entry_id).unwrap_or_default(),
                    authors: authors.remove(&row.entry_id).unwrap_or_default(),
                    row,
                }
            })
            .collect())
    }
}

use autoxide_kernel::pagination::{Page, PageRequest, SortValue, fetch_page, fetch_page_by};
use autoxide_kernel::tenancy::TenantContext;

/// Bộ lọc danh sách bài công khai.
#[derive(Debug, Clone, Default)]
pub(crate) struct PostFilter {
    pub locale: String,
    pub term_id: Option<Uuid>,
    pub author_id: Option<Uuid>,
    pub search: Option<String>,
}

impl ContentService {
    /// Bài đã đăng của một ngôn ngữ, mới nhất trước (theo ngày đăng hiển thị).
    pub(crate) async fn public_posts(
        &self,
        filter: &PostFilter,
        page: &PageRequest,
    ) -> Result<Page<Published>, Error> {
        let mut select = published::Entity::find()
            .filter(published::Column::Kind.eq("post"))
            .filter(published::Column::Locale.eq(filter.locale.as_str()));
        if let Some(term) = filter.term_id {
            select = select.filter(crate::taxonomy::terms::published_has_term(term));
        }
        if let Some(author) = filter.author_id {
            select = select.filter(crate::taxonomy::authors::published_has_author(author));
        }
        if let Some(search) = filter.search.as_deref().filter(|search| !search.trim().is_empty()) {
            select = select
                .filter(autoxide_kernel::search::contains(published::Column::SearchKey, search));
        }
        let key = |row: &Published| SortValue::Time(row.published_at);
        let read = self.database.read();
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
        locale: Option<&str>,
        page: &PageRequest,
    ) -> Result<Page<Published>, Error> {
        let mut select = published::Entity::find();
        if let Some(locale) = locale {
            select = select.filter(published::Column::Locale.eq(locale));
        }
        fetch_page(
            select,
            published::Column::EntryId,
            |row| row.entry_id,
            page,
            self.database.read(),
        )
        .await
    }

    /// `limit` bài mới nhất của `locale`, cho feed.
    pub(crate) async fn feed(&self, locale: &str, limit: u32) -> Result<Vec<Published>, Error> {
        let filter = PostFilter { locale: locale.to_owned(), ..Default::default() };
        let page = PageRequest::first(limit, autoxide_kernel::pagination::Order::Desc);
        Ok(self.public_posts(&filter, &page).await?.data)
    }

    /// Các bản dịch đang công khai của từng mục (ngôn ngữ, đường dẫn), cho sitemap.
    pub(crate) async fn translations_of(
        &self,
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
            .filter(published::Column::TranslationGroupId.is_in(groups))
            .all(self.database.read())
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
