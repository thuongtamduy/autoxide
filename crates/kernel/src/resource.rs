//! Sổ đối tượng trung lập (D-123): module đăng ký một loại đối tượng (`<module>.<loại>`, vd
//! `content.entry`) kèm bộ tra; module khác hỏi "đối tượng này có không, có công khai không, tên
//! và đường dẫn là gì" mà không phụ thuộc module sở hữu và không đọc bảng của nó.
//!
//! - `comments` kiểm đích của bình luận; `newsletter` lấy tiêu đề, tóm tắt, đường dẫn bài mới.
//! - Đăng ký: `context.resource_type("content.entry", resolver)?` trong `Module::start`.
//! - Tra: `context.resources()` (lấy lúc khởi động, giữ lại) rồi `resolve(loại, id)`. Sổ có cả
//!   các loại do module khởi động SAU đăng ký.

use std::sync::{Arc, RwLock};

use uuid::Uuid;

use crate::error::Error;

/// Điều module khác được biết về một đối tượng. Dựng bằng [`Resource::new`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Resource {
    /// Người lạ có xem được không (vd bài đã đăng).
    pub public: bool,
    /// Tiêu đề và tóm tắt người lạ thấy; rỗng khi `public` là `false` (bộ tra không đưa nội dung
    /// chưa công khai ra ngoài module của nó).
    pub title: String,
    pub summary: String,
    /// Đường dẫn công khai (không có tên miền), khi có.
    pub url: Option<String>,
    /// Ngôn ngữ (BCP 47), khi có.
    pub locale: Option<String>,
    /// Người dùng "sở hữu" đối tượng (vd tác giả), để báo cho họ khi cần.
    pub owner_user_ids: Vec<Uuid>,
}

impl Resource {
    /// A private object titled `title`, with no summary, path, language or owners.
    pub fn new(title: impl Into<String>) -> Self {
        Resource {
            public: false,
            title: title.into(),
            summary: String::new(),
            url: None,
            locale: None,
            owner_user_ids: Vec::new(),
        }
    }

    /// Sets whether strangers may see the object.
    pub const fn public(mut self, public: bool) -> Self {
        self.public = public;
        self
    }

    /// Sets the summary strangers see.
    pub fn summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = summary.into();
        self
    }

    /// Sets the public path (without the domain).
    pub fn url(mut self, url: Option<String>) -> Self {
        self.url = url;
        self
    }

    /// Sets the language (BCP 47).
    pub fn locale(mut self, locale: Option<String>) -> Self {
        self.locale = locale;
        self
    }

    /// Sets the users who own the object.
    pub fn owner_user_ids(mut self, owner_user_ids: Vec<Uuid>) -> Self {
        self.owner_user_ids = owner_user_ids;
        self
    }
}

/// Bộ tra một loại đối tượng.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait ResourceResolver: Send + Sync {
    /// Đối tượng `id`; `None` khi không có.
    async fn resolve(&self, id: &str) -> Result<Option<Resource>, Error>;
}

type Entry = (&'static str, Arc<dyn ResourceResolver>);

/// Sổ các loại đối tượng. Clone rẻ: mọi bản clone thấy cùng một sổ.
#[derive(Clone, Default)]
pub struct ResourceDirectory {
    // Ghi chỉ lúc khởi động; đọc mỗi lần tra. Không giữ khoá qua `await`.
    entries: Arc<RwLock<Vec<Entry>>>,
}

impl std::fmt::Debug for ResourceDirectory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResourceDirectory").field("types", &self.types()).finish()
    }
}

impl ResourceDirectory {
    /// Đăng ký loại `kind` của module `module` (tên phải là `<module>.<loại>`). Module dùng
    /// [`StartContext::resource_type`](crate::module::StartContext::resource_type).
    pub fn register(
        &self,
        module: &str,
        kind: &'static str,
        resolver: Arc<dyn ResourceResolver>,
    ) -> Result<(), Error> {
        let valid =
            kind.strip_prefix(module).and_then(|rest| rest.strip_prefix('.')).is_some_and(|rest| {
                !rest.is_empty()
                    && rest
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            });
        if !valid {
            return Err(Error::internal(format!(
                "module `{module}` registers resource type `{kind}`; it must be `{module}.<lower_case_name>`"
            )));
        }
        let mut entries = self.entries.write().map_err(|_| poisoned())?;
        if entries.iter().any(|(name, _)| *name == kind) {
            return Err(Error::internal(format!("resource type `{kind}` is registered twice")));
        }
        entries.push((kind, resolver));
        Ok(())
    }

    /// Các loại đã đăng ký.
    pub fn types(&self) -> Vec<&'static str> {
        self.entries
            .read()
            .map(|entries| entries.iter().map(|(kind, _)| *kind).collect())
            .unwrap_or_default()
    }

    /// Có bộ tra cho loại `kind` không.
    pub fn knows(&self, kind: &str) -> bool {
        self.types().contains(&kind)
    }

    /// Đối tượng `id` thuộc loại `kind`. `Ok(None)` khi không có đối tượng đó; lỗi khi không
    /// có bộ tra cho loại này (hỏi [`knows`](Self::knows) trước nếu loại có thể chưa có).
    pub async fn resolve(&self, kind: &str, id: &str) -> Result<Option<Resource>, Error> {
        let resolver = {
            let entries = self.entries.read().map_err(|_| poisoned())?;
            entries.iter().find(|(name, _)| *name == kind).map(|(_, resolver)| resolver.clone())
        };
        let resolver = resolver
            .ok_or_else(|| Error::internal(format!("no module resolves resource type `{kind}`")))?;
        resolver.resolve(id).await
    }
}

fn poisoned() -> Error {
    Error::internal("the resource directory is unusable after a panic")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed;

    #[async_trait::async_trait]
    impl ResourceResolver for Fixed {
        async fn resolve(&self, id: &str) -> Result<Option<Resource>, Error> {
            Ok((id == "1").then(|| Resource {
                public: true,
                title: "One".to_owned(),
                ..Resource::default()
            }))
        }
    }

    #[tokio::test]
    async fn dang_ky_va_tra() {
        let directory = ResourceDirectory::default();
        let early = directory.clone();
        directory.register("content", "content.entry", Arc::new(Fixed)).unwrap();
        assert!(directory.register("content", "content.entry", Arc::new(Fixed)).is_err(), "twice");
        assert!(
            directory.register("content", "media.item", Arc::new(Fixed)).is_err(),
            "other module"
        );
        assert!(directory.register("content", "content.Bad", Arc::new(Fixed)).is_err());
        assert!(early.knows("content.entry"), "clones see later registrations");
        assert_eq!(early.resolve("content.entry", "1").await.unwrap().unwrap().title, "One");
        assert!(early.resolve("content.entry", "2").await.unwrap().is_none());
        assert!(early.resolve("forms.form", "1").await.is_err());
    }
}
