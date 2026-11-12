//! Đọc bản công khai cho người đọc (không cần đăng nhập): chỉ từ `content_published`, nên
//! không bao giờ lộ bản nháp (D-102). Ghép thêm ảnh (qua `media`: ảnh không còn công khai coi
//! như không có), các bản dịch đang công khai (cho `hreflang`), danh mục, thẻ, tác giả.

use std::collections::{BTreeSet, HashMap};

use autoxide_kernel::error::Error;
use autoxide_media::api::{MediaRef, MediaVisibility};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use uuid::Uuid;

use crate::authors::AuthorRef;
use crate::entity::published;
use crate::publishing::Published;
use crate::service::ContentService;
use crate::terms::TermRef;

/// Một mục công khai kèm những gì website cần để hiển thị.
#[derive(Debug, Clone)]
pub(crate) struct PublicView {
    pub row: Published,
    pub featured: Option<MediaRef>,
    pub og: Option<MediaRef>,
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
        let media_ids: Vec<Uuid> = rows
            .iter()
            .flat_map(|row| [row.featured_media_id, row.og_media_id])
            .flatten()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let mut media: HashMap<Uuid, MediaRef> = HashMap::new();
        for chunk in media_ids.chunks(autoxide_media::api::MAX_IDS) {
            for item in self.media.find_many(chunk).await? {
                if item.visibility == MediaVisibility::Public {
                    media.insert(item.id, item);
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
        let ids: Vec<Uuid> = rows.iter().map(|row| row.entry_id).collect();
        let mut terms =
            crate::terms::terms_of(self.database.read(), &ids, &self.config.default_locale, &rows)
                .await?;
        let mut authors = crate::authors::authors_of(self.database.read(), &ids).await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let translations = siblings
                    .iter()
                    .filter(|other| {
                        other.translation_group_id == row.translation_group_id
                            && other.entry_id != row.entry_id
                    })
                    .map(|other| (other.locale.clone(), other.path.clone()))
                    .collect();
                PublicView {
                    featured: row.featured_media_id.and_then(|id| media.get(&id).cloned()),
                    og: row.og_media_id.and_then(|id| media.get(&id).cloned()),
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
            select = select.filter(crate::terms::published_has_term(term));
        }
        if let Some(author) = filter.author_id {
            select = select.filter(crate::authors::published_has_author(author));
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
