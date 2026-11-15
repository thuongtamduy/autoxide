//! Sổ đối tượng trung lập (D-123): module đăng ký một loại đối tượng (`<module>.<loại>`, vd
//! `content.entry`) kèm bộ tra; module khác hỏi "đối tượng này có không, có công khai không, tên
//! và đường dẫn là gì" mà không phụ thuộc module sở hữu và không đọc bảng của nó.
//!
//! - `comments` kiểm đích của bình luận; `newsletter` lấy tiêu đề, tóm tắt, đường dẫn bài mới.
//! - Đăng ký: `context.resource_type("content.entry", resolver)?` trong `Module::start`.
//! - Tra: `context.resources()` (lấy lúc khởi động, giữ lại) rồi `resolve(loại, id)`. Sổ có cả
//!   các loại do module khởi động SAU đăng ký.
//! - Tra nhiều đối tượng một lần, theo ngôn ngữ: `resolve_many(loại, ids, locale)`; nhóm con của
//!   loại có cây (vd danh mục): `children(loại, id, locale, limit)` (D-238, `menus` dùng).
//!
//! Hai thứ đi kèm (D-202), cũng trung lập với nghiệp vụ:
//!
//! - **Event vòng đời** ([`ResourceEvent`]): module sở hữu khai qua `Module::resource_events`
//!   event nào nghĩa là đối tượng "thôi nhận tương tác mới" ([`ResourceChange::Closed`]) hay "đã
//!   gỡ" ([`ResourceChange::Removed`]). Module gắn dữ liệu vào đối tượng của module khác (vd
//!   `comments`) nghe các event đó mà không cần biết module sở hữu là ai.
//! - **Nhãn tác giả** ([`BadgeResolver`]): module đăng ký bộ tính nhãn cho một loại đối tượng
//!   (`context.badge_source(…)`), vd `seller` cho người quản lý sản phẩm, `verified_buyer` cho
//!   người đã nhận hàng. Bên hỏi nhận mã nhãn ổn định, không biết module nào tính ra.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use uuid::Uuid;

use crate::access::Principal;
use crate::error::Error;

/// Điều một event của module sở hữu nghĩa là với đối tượng của nó.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResourceChange {
    /// Không nhận tương tác mới nữa (vd bài bị lưu trữ); thứ đã có vẫn được xem.
    Closed,
    /// Đã gỡ (vd bị xoá): thôi nhận và ẩn mọi thứ gắn vào nó.
    Removed,
}

/// Một event vòng đời của một loại đối tượng: event `event` (do chính module sở hữu khai trong
/// `Module::events`) với chủ thể `<kind>:<id>` nghĩa là `change` cho đối tượng `id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResourceEvent {
    pub event: &'static str,
    pub kind: &'static str,
    pub change: ResourceChange,
}

impl ResourceEvent {
    /// Event `event` báo `change` cho đối tượng loại `kind`.
    pub const fn new(event: &'static str, kind: &'static str, change: ResourceChange) -> Self {
        ResourceEvent { event, kind, change }
    }

    /// ID của đối tượng trong chủ thể `<kind>:<id>` của một event; `None` khi chủ thể không
    /// thuộc loại này.
    pub fn id_in<'a>(&self, subject: &'a str) -> Option<&'a str> {
        subject
            .strip_prefix(self.kind)
            .and_then(|rest| rest.strip_prefix(':'))
            .filter(|id| !id.is_empty())
    }
}

/// Bộ tính nhãn của tác giả trên một loại đối tượng.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait BadgeResolver: Send + Sync {
    /// Các nhãn `author` có trên đối tượng `id` (chỉ trong số nhãn đã khai lúc đăng ký; nhãn
    /// khác bị bỏ). Đối tượng không có thì trả rỗng.
    async fn badges(&self, id: &str, author: &Principal) -> Result<Vec<&'static str>, Error>;
}

/// Bộ tính nhãn đã đăng ký: module, loại đối tượng, các nhãn nó có thể trả, bộ tính.
type BadgeEntry = (&'static str, &'static str, &'static [&'static str], Arc<dyn BadgeResolver>);

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
    /// Ảnh đại diện (ID trong thư viện `media`), khi có. Bên hỏi tự kiểm ảnh có công khai không.
    pub image: Option<Uuid>,
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
            image: None,
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

    /// Sets the representative image (an ID in the media library).
    pub const fn image(mut self, image: Option<Uuid>) -> Self {
        self.image = image;
        self
    }
}

/// Chỉ tới một đối tượng trong sổ: loại (`<module>.<loại>`) và ID. Dựng bằng [`ResourceRef::new`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct ResourceRef {
    pub kind: String,
    pub id: String,
}

impl ResourceRef {
    /// The object `id` of type `kind`.
    pub fn new(kind: impl Into<String>, id: impl Into<String>) -> Self {
        ResourceRef { kind: kind.into(), id: id.into() }
    }
}

/// Bộ tra một loại đối tượng.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait ResourceResolver: Send + Sync {
    /// Đối tượng `id`; `None` khi không có.
    async fn resolve(&self, id: &str) -> Result<Option<Resource>, Error>;

    /// Nhiều đối tượng một lần, với tiêu đề, đường dẫn theo ngôn ngữ `locale` khi loại có bản
    /// dịch (thiếu bản dịch thì bộ tra tự chọn ngôn ngữ mặc định của nó). Đối tượng không có thì
    /// vắng trong kết quả. Mặc định gọi [`resolve`](Self::resolve) từng cái, bỏ qua `locale`;
    /// loại hay được tra theo lô (vd mục menu) nên cài bằng một câu query.
    async fn resolve_many(
        &self,
        ids: &[String],
        locale: Option<&str>,
    ) -> Result<HashMap<String, Resource>, Error> {
        let _ = locale;
        let mut found = HashMap::new();
        for id in ids {
            if found.contains_key(id) {
                continue;
            }
            if let Some(resource) = self.resolve(id).await? {
                found.insert(id.clone(), resource);
            }
        }
        Ok(found)
    }

    /// Loại này có cây không (vd danh mục có danh mục con): có thì [`children`](Self::children)
    /// trả nhóm con. Mặc định `false`.
    fn has_children(&self) -> bool {
        false
    }

    /// Tối đa `limit` đối tượng con trực tiếp của `id`, theo thứ tự hiển thị của module sở hữu,
    /// chỉ những cái người lạ xem được. Mặc định rỗng.
    async fn children(
        &self,
        id: &str,
        locale: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ResourceRef>, Error> {
        let _ = (id, locale, limit);
        Ok(Vec::new())
    }
}

type Entry = (&'static str, Arc<dyn ResourceResolver>);

/// Sổ các loại đối tượng. Clone rẻ: mọi bản clone thấy cùng một sổ.
#[derive(Clone, Default)]
pub struct ResourceDirectory {
    // Ghi chỉ lúc khởi động; đọc mỗi lần tra. Không giữ khoá qua `await`.
    entries: Arc<RwLock<Vec<Entry>>>,
    badges: Arc<RwLock<Vec<BadgeEntry>>>,
}

impl std::fmt::Debug for ResourceDirectory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResourceDirectory").field("types", &self.types()).finish()
    }
}

/// Mã nhãn: chữ thường, số, `_`, bắt đầu bằng chữ, tối đa 32 ký tự (vd `verified_buyer`).
fn is_badge(code: &str) -> bool {
    code.len() <= 32
        && code.starts_with(|c: char| c.is_ascii_lowercase())
        && code.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
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
        self.resolver(kind)?.resolve(id).await
    }

    /// Nhiều đối tượng loại `kind` một lần, theo ngôn ngữ `locale`: ID → đối tượng; ID không có
    /// thì vắng. Lỗi khi không có bộ tra cho loại này. Có thể gọi module khác: gọi NGOÀI
    /// transaction ghi.
    pub async fn resolve_many(
        &self,
        kind: &str,
        ids: &[String],
        locale: Option<&str>,
    ) -> Result<HashMap<String, Resource>, Error> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        self.resolver(kind)?.resolve_many(ids, locale).await
    }

    /// Loại `kind` có cây không (có [`children`](Self::children)); `false` cả khi chưa đăng ký.
    pub fn has_children(&self, kind: &str) -> bool {
        self.resolver(kind).is_ok_and(|resolver| resolver.has_children())
    }

    /// Tối đa `limit` đối tượng con công khai của `id` (loại `kind`). Lỗi khi không có bộ tra
    /// cho loại này.
    pub async fn children(
        &self,
        kind: &str,
        id: &str,
        locale: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ResourceRef>, Error> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut found = self.resolver(kind)?.children(id, locale, limit).await?;
        found.truncate(limit);
        Ok(found)
    }

    fn resolver(&self, kind: &str) -> Result<Arc<dyn ResourceResolver>, Error> {
        let resolver = {
            let entries = self.entries.read().map_err(|_| poisoned())?;
            entries.iter().find(|(name, _)| *name == kind).map(|(_, resolver)| resolver.clone())
        };
        resolver
            .ok_or_else(|| Error::internal(format!("no module resolves resource type `{kind}`")))
    }

    /// Đăng ký bộ tính nhãn của module `module` cho loại `kind` — loại của module nào cũng
    /// được (vd `orders` tính `verified_buyer` cho `catalog.product`). `codes`: mọi nhãn bộ
    /// này có thể trả. Một nhãn của một loại chỉ do một module tính. Module dùng
    /// [`StartContext::badge_source`](crate::module::StartContext::badge_source).
    pub fn register_badges(
        &self,
        module: &'static str,
        kind: &'static str,
        codes: &'static [&'static str],
        resolver: Arc<dyn BadgeResolver>,
    ) -> Result<(), Error> {
        if codes.is_empty() || !codes.iter().all(|code| is_badge(code)) {
            return Err(Error::internal(format!(
                "module `{module}` registers badges {codes:?} for `{kind}`; list at least one, \
                 each lower_case letters, digits and `_`, at most 32 characters"
            )));
        }
        let mut entries = self.badges.write().map_err(|_| poisoned())?;
        for (owner, known, taken, _) in entries.iter() {
            if let Some(code) = codes.iter().find(|code| *known == kind && taken.contains(code)) {
                return Err(Error::internal(format!(
                    "badge `{code}` of `{kind}` is registered by modules `{owner}` and `{module}`"
                )));
            }
        }
        entries.push((module, kind, codes, resolver));
        Ok(())
    }

    /// Mọi nhãn có thể có của loại `kind`, theo thứ tự đăng ký.
    pub fn badge_codes(&self, kind: &str) -> Vec<&'static str> {
        self.badges
            .read()
            .map(|entries| {
                entries
                    .iter()
                    .filter(|(_, known, _, _)| *known == kind)
                    .flat_map(|(_, _, codes, _)| codes.iter().copied())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Nhãn của `author` trên đối tượng `id` loại `kind`, gom từ mọi bộ tính của loại đó, theo
    /// thứ tự đăng ký, không trùng. Loại không có bộ tính nào thì rỗng. Có thể gọi module khác:
    /// gọi NGOÀI transaction ghi.
    pub async fn badges(
        &self,
        kind: &str,
        id: &str,
        author: &Principal,
    ) -> Result<Vec<&'static str>, Error> {
        let sources: Vec<(&'static [&'static str], Arc<dyn BadgeResolver>)> = {
            let entries = self.badges.read().map_err(|_| poisoned())?;
            entries
                .iter()
                .filter(|(_, known, _, _)| *known == kind)
                .map(|(_, _, codes, resolver)| (*codes, resolver.clone()))
                .collect()
        };
        let mut found = Vec::new();
        for (codes, resolver) in sources {
            for code in resolver.badges(id, author).await? {
                if let Some(code) = codes.iter().find(|known| **known == code)
                    && !found.contains(code)
                {
                    found.push(*code);
                }
            }
        }
        Ok(found)
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

    /// Loại có cây: `p` có hai con `c1`, `c2`; tra theo lô và theo ngôn ngữ.
    struct Tree;

    #[async_trait::async_trait]
    impl ResourceResolver for Tree {
        async fn resolve(&self, id: &str) -> Result<Option<Resource>, Error> {
            Ok(Some(Resource::new(id.to_uppercase()).public(true)))
        }

        async fn resolve_many(
            &self,
            ids: &[String],
            locale: Option<&str>,
        ) -> Result<HashMap<String, Resource>, Error> {
            let suffix = locale.unwrap_or("-");
            Ok(ids
                .iter()
                .map(|id| (id.clone(), Resource::new(format!("{id}@{suffix}")).public(true)))
                .collect())
        }

        fn has_children(&self) -> bool {
            true
        }

        async fn children(
            &self,
            id: &str,
            _: Option<&str>,
            _: usize,
        ) -> Result<Vec<ResourceRef>, Error> {
            Ok(if id == "p" {
                vec![ResourceRef::new("shop.tree", "c1"), ResourceRef::new("shop.tree", "c2")]
            } else {
                Vec::new()
            })
        }
    }

    #[tokio::test]
    async fn tra_theo_lo_va_nhom_con() {
        let directory = ResourceDirectory::default();
        directory.register("content", "content.entry", Arc::new(Fixed)).unwrap();
        directory.register("shop", "shop.tree", Arc::new(Tree)).unwrap();

        // Mặc định: tra từng cái, bỏ ID không có, không trùng.
        let ids = vec!["1".to_owned(), "2".to_owned(), "1".to_owned()];
        let found = directory.resolve_many("content.entry", &ids, Some("vi")).await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found["1"].title, "One");
        assert!(directory.resolve_many("forms.form", &ids, None).await.is_err());
        assert!(directory.resolve_many("forms.form", &[], None).await.unwrap().is_empty());

        // Cài riêng: theo ngôn ngữ.
        let found = directory.resolve_many("shop.tree", &ids, Some("vi")).await.unwrap();
        assert_eq!(found["2"].title, "2@vi");

        assert!(!directory.has_children("content.entry"), "default: no tree");
        assert!(directory.has_children("shop.tree"));
        assert!(!directory.has_children("forms.form"), "unknown kinds have no tree");
        assert!(directory.children("content.entry", "1", None, 5).await.unwrap().is_empty());
        let children = directory.children("shop.tree", "p", None, 1).await.unwrap();
        assert_eq!(children, [ResourceRef::new("shop.tree", "c1")], "cut at the limit");
        assert!(directory.children("shop.tree", "p", None, 0).await.unwrap().is_empty());
        assert!(directory.children("forms.form", "p", None, 5).await.is_err());

        let image = Uuid::now_v7();
        assert_eq!(Resource::new("A").image(Some(image)).image, Some(image));
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

    struct Seller;

    #[async_trait::async_trait]
    impl BadgeResolver for Seller {
        async fn badges(&self, id: &str, author: &Principal) -> Result<Vec<&'static str>, Error> {
            Ok(if id == "1" && author.allows("catalog.manage") {
                vec!["seller", "unknown"]
            } else {
                Vec::new()
            })
        }
    }

    struct Buyer;

    #[async_trait::async_trait]
    impl BadgeResolver for Buyer {
        async fn badges(&self, _: &str, _: &Principal) -> Result<Vec<&'static str>, Error> {
            Ok(vec!["verified_buyer", "verified_buyer"])
        }
    }

    #[tokio::test]
    async fn nhan_gom_tu_moi_module_va_chi_nhan_da_khai() {
        let directory = ResourceDirectory::default();
        directory
            .register_badges("catalog", "catalog.product", &["seller"], Arc::new(Seller))
            .unwrap();
        directory
            .register_badges("orders", "catalog.product", &["verified_buyer"], Arc::new(Buyer))
            .unwrap();
        assert!(
            directory
                .register_badges("shop", "catalog.product", &["seller"], Arc::new(Seller))
                .is_err(),
            "one module per badge of a kind"
        );
        assert!(
            directory.register_badges("shop", "shop.item", &["Bad"], Arc::new(Seller)).is_err()
        );
        assert!(directory.register_badges("shop", "shop.item", &[], Arc::new(Seller)).is_err());
        assert_eq!(directory.badge_codes("catalog.product"), ["seller", "verified_buyer"]);

        let manager = Principal::new(Uuid::now_v7(), ["catalog.manage".to_owned()]);
        let buyer = Principal::new(Uuid::now_v7(), Vec::<String>::new());
        let found = directory.badges("catalog.product", "1", &manager).await.unwrap();
        assert_eq!(found, ["seller", "verified_buyer"], "undeclared badges are dropped");
        assert_eq!(
            directory.badges("catalog.product", "1", &buyer).await.unwrap(),
            ["verified_buyer"]
        );
        assert!(directory.badges("content.entry", "1", &manager).await.unwrap().is_empty());
    }

    #[test]
    fn chu_the_cua_event_vong_doi() {
        let event =
            ResourceEvent::new("content.entry_deleted", "content.entry", ResourceChange::Removed);
        assert_eq!(event.id_in("content.entry:abc"), Some("abc"));
        assert_eq!(event.id_in("content.entry:"), None);
        assert_eq!(event.id_in("content.entryx:abc"), None);
        assert_eq!(event.id_in("media.item:abc"), None);
    }
}
