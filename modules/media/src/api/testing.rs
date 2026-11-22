//! Thư viện media giả dùng chung cho test của module khác (feature `testing`, chỉ khai ở
//! `[dev-dependencies]`; quy ước D-20261122-module-test-fakes).
//!
//! Giữ item trong bộ nhớ: ID không có (hay đã xoá) thì không trả, như module `media` thật. Mặc định
//! một item là ảnh `image/jpeg` 800×800, `alt` rỗng, chưa có người tải lên.
//!
//! ```ignore
//! let media = Arc::new(FakeMedia::new());
//! let photo = media.add(true);
//! let app = TestApp::builder(test).provide::<dyn Media>(media.clone())…;
//! ```

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::TenantContext;
use uuid::Uuid;

use super::{AiImage, Media, MediaRef, MediaVisibility};

#[derive(Clone)]
struct Item {
    media: MediaRef,
    uploader: Option<Uuid>,
    /// `alt` theo ngôn ngữ (`find_many_in`).
    alts: HashMap<String, String>,
    ai_image: Option<AiImage>,
}

#[derive(Default)]
struct State {
    /// Theo thứ tự thêm.
    items: Vec<Item>,
    /// `find_many_in` trả `alt` là `[<ngôn ngữ>]` khi item không có bản dịch: test thấy ngay ảnh
    /// được tra theo ngôn ngữ nào.
    tag_locale: bool,
}

/// Thư viện media giả. Công bố bằng `TestApp::builder(…).provide::<dyn Media>(media.clone())`.
#[derive(Default)]
pub struct FakeMedia {
    state: Mutex<State>,
}

// Khoá `Mutex` mà không `unwrap`: test khác panic giữa chừng thì vẫn đọc tiếp dữ liệu bên trong.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Item mặc định mang ID `id`: ảnh JPEG 800×800, công khai hay riêng tư.
pub fn image(id: Uuid, public: bool) -> MediaRef {
    MediaRef {
        id,
        visibility: if public { MediaVisibility::Public } else { MediaVisibility::Private },
        content_type: "image/jpeg".to_owned(),
        width: Some(800),
        height: Some(800),
        alt: String::new(),
        alt_ai_assisted: false,
        url: format!("/api/v1/media/{id}/content?v=1"),
        variants: Vec::new(),
    }
}

impl FakeMedia {
    pub fn new() -> Self {
        FakeMedia::default()
    }

    /// Bật: `find_many_in` trả `alt` là `[<ngôn ngữ>]` cho item không có bản dịch ở ngôn ngữ đó.
    #[must_use]
    pub fn tagging_locale(self) -> Self {
        lock(&self.state).tag_locale = true;
        self
    }

    /// Thêm ảnh mặc định (công khai hay riêng tư); trả ID mới.
    pub fn add(&self, public: bool) -> Uuid {
        self.add_with(public, |_| {})
    }

    /// Như [`add`](Self::add), sửa item trước khi thêm (kích thước, `alt`, loại…).
    pub fn add_with(&self, public: bool, edit: impl FnOnce(&mut MediaRef)) -> Uuid {
        let mut media = image(Uuid::now_v7(), public);
        edit(&mut media);
        let id = media.id;
        self.insert(media);
        id
    }

    /// Thêm (hay thay) item mang đúng ID của `media`.
    pub fn insert(&self, media: MediaRef) {
        let mut state = lock(&self.state);
        if let Some(found) = state.items.iter_mut().find(|item| item.media.id == media.id) {
            found.media = media;
            return;
        }
        state.items.push(Item { media, uploader: None, alts: HashMap::new(), ai_image: None });
    }

    /// Ảnh riêng tư mới do người `user` tải lên.
    pub fn upload(&self, user: Uuid) -> Uuid {
        let id = self.add(false);
        self.edit(id, |item| item.uploader = Some(user));
        id
    }

    /// Đổi công khai / riêng tư của item `id`.
    pub fn set_visibility(&self, id: Uuid, public: bool) {
        self.edit(id, |item| {
            item.media.visibility =
                if public { MediaVisibility::Public } else { MediaVisibility::Private };
        });
    }

    /// `alt` của item `id` ở ngôn ngữ `locale` (`find_many_in`).
    pub fn set_alt(&self, id: Uuid, locale: &str, alt: &str) {
        self.edit(id, |item| {
            item.alts.insert(locale.to_owned(), alt.to_owned());
        });
    }

    /// Ảnh `ai_image` trả cho item `id` (mặc định `None`, như ảnh không gửi được cho AI).
    pub fn set_ai_image(&self, id: Uuid, image: AiImage) {
        self.edit(id, |item| item.ai_image = Some(image));
    }

    /// Xoá item `id` (như đưa vào thùng rác).
    pub fn remove(&self, id: Uuid) {
        lock(&self.state).items.retain(|item| item.media.id != id);
    }

    fn edit(&self, id: Uuid, change: impl FnOnce(&mut Item)) {
        if let Some(item) = lock(&self.state).items.iter_mut().find(|item| item.media.id == id) {
            change(item);
        }
    }

    fn found(&self, ids: &[Uuid]) -> Vec<Item> {
        let state = lock(&self.state);
        ids.iter()
            .filter_map(|id| state.items.iter().find(|item| item.media.id == *id).cloned())
            .collect()
    }
}

#[async_trait::async_trait]
impl Media for FakeMedia {
    async fn find_many(&self, _: &TenantContext, ids: &[Uuid]) -> Result<Vec<MediaRef>, Error> {
        Ok(self.found(ids).into_iter().map(|item| item.media).collect())
    }

    async fn find_many_in(
        &self,
        _: &TenantContext,
        ids: &[Uuid],
        locale: &str,
    ) -> Result<Vec<MediaRef>, Error> {
        let tag = lock(&self.state).tag_locale;
        Ok(self
            .found(ids)
            .into_iter()
            .map(|item| {
                let mut media = item.media;
                if let Some(alt) = item.alts.get(locale) {
                    media.alt = alt.clone();
                } else if tag {
                    media.alt = format!("[{locale}]");
                }
                media
            })
            .collect())
    }

    async fn uploaders(
        &self,
        _: &TenantContext,
        ids: &[Uuid],
    ) -> Result<Vec<(Uuid, Option<Uuid>)>, Error> {
        Ok(self.found(ids).into_iter().map(|item| (item.media.id, item.uploader)).collect())
    }

    async fn ai_image(&self, _: &TenantContext, id: Uuid) -> Result<Option<AiImage>, Error> {
        Ok(self.found(&[id]).into_iter().next().and_then(|item| item.ai_image))
    }
}
