//! Neutral resource registry (D-123): a module registers a kind of object (`<module>.<kind>`, e.g.
//! `content.entry`) together with a resolver; other modules ask "does this object exist, is it public, what are
//! its name and path" without depending on the owning module and without reading its tables.
//!
//! - `comments` validates the target of a comment; `newsletter` gets the title, summary and path of new articles.
//! - Register: `context.resource_type("content.entry", resolver)?` in `Module::start`.
//! - Resolve: `context.resources()` (obtained at startup and kept) then `resolve(kind, id)`. The registry also
//!   contains kinds registered by modules that start AFTER.
//! - Resolve many objects at once, by language: `resolve_many(kind, ids, locale)`; the children of a kind
//!   that has a tree (e.g. categories): `children(kind, id, locale, limit)` (D-238, used by `menus`).
//!
//! Two companions (D-202), also business-neutral:
//!
//! - **Lifecycle events** ([`ResourceEvent`]): the owning module declares through `Module::resource_events`
//!   which event means an object "no longer accepts new interactions" ([`ResourceChange::Closed`]) or "was
//!   removed" ([`ResourceChange::Removed`]). A module that attaches data to another module's objects (e.g.
//!   `comments`) listens to those events without needing to know who the owning module is.
//! - **Author badges** ([`BadgeResolver`]): a module registers a badge computer for a kind of object
//!   (`context.badge_source(...)`), e.g. `seller` for the person managing a product, `verified_buyer` for
//!   someone who received the goods. The asker receives stable badge codes and does not know which module
//!   computed them.

use crate::time::Timestamp;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use uuid::Uuid;

use crate::access::Principal;
use crate::error::Error;
use crate::tenancy::TenantContext;

/// What an event of the owning module means for its object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResourceChange {
    /// No longer accepts new interactions (e.g. the article is archived); existing ones stay viewable.
    Closed,
    /// Removed (e.g. deleted): stops accepting interactions and hides everything attached to it.
    Removed,
}

/// A lifecycle event of a kind of object: event `event` (declared by the owning module itself in
/// `Module::events`) with subject `<kind>:<id>` means `change` for the object `id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResourceEvent {
    pub event: &'static str,
    pub kind: &'static str,
    pub change: ResourceChange,
}

impl ResourceEvent {
    /// Event `event` reports `change` for objects of kind `kind`.
    pub const fn new(event: &'static str, kind: &'static str, change: ResourceChange) -> Self {
        ResourceEvent { event, kind, change }
    }

    /// ID of the object in the `<kind>:<id>` subject of an event; `None` when the subject is not of
    /// this kind.
    pub fn id_in<'a>(&self, subject: &'a str) -> Option<&'a str> {
        subject
            .strip_prefix(self.kind)
            .and_then(|rest| rest.strip_prefix(':'))
            .filter(|id| !id.is_empty())
    }
}

/// Computer of an author's badges on a kind of object.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait BadgeResolver: Send + Sync {
    /// The badges `author` has on object `id` (only among the badges declared at registration; other
    /// badges are dropped). Empty when the object has none.
    async fn badges(
        &self,
        tenant: &TenantContext,
        id: &str,
        author: &Principal,
    ) -> Result<Vec<&'static str>, Error>;
}

/// A registered badge computer: module, kind of object, the badges it can return, the computer.
type BadgeEntry = (&'static str, &'static str, &'static [&'static str], Arc<dyn BadgeResolver>);

/// What other modules may know about an object. Build it with [`Resource::new`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Resource {
    /// Whether a stranger may see it (e.g. the article is published).
    pub public: bool,
    /// Title and summary that strangers see; empty when `public` is `false` (the resolver does not expose
    /// non-public content outside its module).
    pub title: String,
    pub summary: String,
    /// Public path (without the domain), when there is one.
    pub url: Option<String>,
    /// Language (BCP 47), when there is one.
    pub locale: Option<String>,
    /// Users who "own" the object (e.g. the author), to notify them when needed.
    pub owner_user_ids: Vec<Uuid>,
    /// Representative image (ID in the `media` library), when there is one. The asker checks itself whether the image is public.
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

/// Points to an object in the registry: kind (`<module>.<kind>`) and ID. Build it with [`ResourceRef::new`].
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

/// Resolver for one kind of object.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait ResourceResolver: Send + Sync {
    /// Object `id` of tenant `tenant`; `None` when it does not exist.
    async fn resolve(&self, tenant: &TenantContext, id: &str) -> Result<Option<Resource>, Error>;

    /// Many objects at once, with titles and paths in language `locale` when the kind has translations (when
    /// a translation is missing the resolver picks its own default language). Objects that do not exist are
    /// absent from the result. The default calls [`resolve`](Self::resolve) one by one and ignores `locale`;
    /// kinds that are often resolved in batches (e.g. menu items) should implement it with a single query.
    async fn resolve_many(
        &self,
        tenant: &TenantContext,
        ids: &[String],
        locale: Option<&str>,
    ) -> Result<HashMap<String, Resource>, Error> {
        let _ = locale;
        let mut found = HashMap::new();
        for id in ids {
            if found.contains_key(id) {
                continue;
            }
            if let Some(resource) = self.resolve(tenant, id).await? {
                found.insert(id.clone(), resource);
            }
        }
        Ok(found)
    }

    /// Whether this kind has a tree (e.g. categories with sub-categories): if so [`children`](Self::children)
    /// returns the children. Defaults to `false`.
    fn has_children(&self) -> bool {
        false
    }

    /// At most `limit` direct children of `id`, in the owning module's display order, only those strangers can
    /// see. Defaults to empty.
    async fn children(
        &self,
        tenant: &TenantContext,
        id: &str,
        locale: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ResourceRef>, Error> {
        let _ = (tenant, id, locale, limit);
        Ok(Vec::new())
    }
}

type Entry = (&'static str, Arc<dyn ResourceResolver>);

/// Registry of kinds of objects. Cheap to clone: every clone sees the same registry.
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

/// Badge code: lowercase letters, digits, `_`, starting with a letter, at most 32 characters (e.g. `verified_buyer`).
fn is_badge(code: &str) -> bool {
    code.len() <= 32
        && code.starts_with(|c: char| c.is_ascii_lowercase())
        && code.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

impl ResourceDirectory {
    /// Registers kind `kind` of module `module` (the name must be `<module>.<kind>`). Modules use
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

    /// The registered kinds.
    pub fn types(&self) -> Vec<&'static str> {
        self.entries
            .read()
            .map(|entries| entries.iter().map(|(kind, _)| *kind).collect())
            .unwrap_or_default()
    }

    /// Whether there is a resolver for kind `kind`.
    pub fn knows(&self, kind: &str) -> bool {
        self.types().contains(&kind)
    }

    /// Object `id` of kind `kind`. `Ok(None)` when no such object exists; an error when there is no resolver
    /// for this kind (ask [`knows`](Self::knows) first if the kind may not be registered yet).
    pub async fn resolve(
        &self,
        tenant: &TenantContext,
        kind: &str,
        id: &str,
    ) -> Result<Option<Resource>, Error> {
        self.resolver(kind)?.resolve(tenant, id).await
    }

    /// Many objects of kind `kind` at once, in language `locale`: ID -> object; IDs that do not exist are
    /// absent. An error when there is no resolver for this kind. May call another module: call it OUTSIDE a
    /// write transaction.
    pub async fn resolve_many(
        &self,
        tenant: &TenantContext,
        kind: &str,
        ids: &[String],
        locale: Option<&str>,
    ) -> Result<HashMap<String, Resource>, Error> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        self.resolver(kind)?.resolve_many(tenant, ids, locale).await
    }

    /// Whether kind `kind` has a tree (has [`children`](Self::children)); `false` also when not registered.
    pub fn has_children(&self, kind: &str) -> bool {
        self.resolver(kind).is_ok_and(|resolver| resolver.has_children())
    }

    /// At most `limit` public children of `id` (kind `kind`). An error when there is no resolver
    /// for this kind.
    pub async fn children(
        &self,
        tenant: &TenantContext,
        kind: &str,
        id: &str,
        locale: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ResourceRef>, Error> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut found = self.resolver(kind)?.children(tenant, id, locale, limit).await?;
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

    /// Registers the badge computer of module `module` for kind `kind`; any module's kind is allowed
    /// (e.g. `orders` computes `verified_buyer` for `catalog.product`). `codes`: every badge this
    /// computer can return. A badge of a kind is computed by only one module. Modules use
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

    /// Every possible badge of kind `kind`, in registration order.
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

    /// Badges of `author` on object `id` of kind `kind`, gathered from every computer of that kind, in
    /// registration order, without duplicates. Empty when the kind has no computer. May call another module:
    /// call it OUTSIDE a write transaction.
    pub async fn badges(
        &self,
        tenant: &TenantContext,
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
            for code in resolver.badges(tenant, id, author).await? {
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
        async fn resolve(
            &self,
            _tenant: &crate::tenancy::TenantContext,
            id: &str,
        ) -> Result<Option<Resource>, Error> {
            Ok((id == "1").then(|| Resource {
                public: true,
                title: "One".to_owned(),
                ..Resource::default()
            }))
        }
    }

    /// Kind with a tree: `p` has two children `c1`, `c2`; resolved in batches and by language.
    struct Tree;

    #[async_trait::async_trait]
    impl ResourceResolver for Tree {
        async fn resolve(
            &self,
            _tenant: &crate::tenancy::TenantContext,
            id: &str,
        ) -> Result<Option<Resource>, Error> {
            Ok(Some(Resource::new(id.to_uppercase()).public(true)))
        }

        async fn resolve_many(
            &self,
            _tenant: &crate::tenancy::TenantContext,
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
            _tenant: &crate::tenancy::TenantContext,
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
        let found = directory
            .resolve_many(&crate::tenancy::test_tenant(), "content.entry", &ids, Some("vi"))
            .await
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found["1"].title, "One");
        assert!(
            directory
                .resolve_many(&crate::tenancy::test_tenant(), "forms.form", &ids, None)
                .await
                .is_err()
        );
        assert!(
            directory
                .resolve_many(&crate::tenancy::test_tenant(), "forms.form", &[], None)
                .await
                .unwrap()
                .is_empty()
        );

        // Cài riêng: theo ngôn ngữ.
        let found = directory
            .resolve_many(&crate::tenancy::test_tenant(), "shop.tree", &ids, Some("vi"))
            .await
            .unwrap();
        assert_eq!(found["2"].title, "2@vi");

        assert!(!directory.has_children("content.entry"), "default: no tree");
        assert!(directory.has_children("shop.tree"));
        assert!(!directory.has_children("forms.form"), "unknown kinds have no tree");
        assert!(
            directory
                .children(&crate::tenancy::test_tenant(), "content.entry", "1", None, 5)
                .await
                .unwrap()
                .is_empty()
        );
        let children = directory
            .children(&crate::tenancy::test_tenant(), "shop.tree", "p", None, 1)
            .await
            .unwrap();
        assert_eq!(children, [ResourceRef::new("shop.tree", "c1")], "cut at the limit");
        assert!(
            directory
                .children(&crate::tenancy::test_tenant(), "shop.tree", "p", None, 0)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            directory
                .children(&crate::tenancy::test_tenant(), "forms.form", "p", None, 5)
                .await
                .is_err()
        );

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
        assert_eq!(
            early
                .resolve(&crate::tenancy::test_tenant(), "content.entry", "1")
                .await
                .unwrap()
                .unwrap()
                .title,
            "One"
        );
        assert!(
            early
                .resolve(&crate::tenancy::test_tenant(), "content.entry", "2")
                .await
                .unwrap()
                .is_none()
        );
        assert!(early.resolve(&crate::tenancy::test_tenant(), "forms.form", "1").await.is_err());
    }

    struct Seller;

    #[async_trait::async_trait]
    impl BadgeResolver for Seller {
        async fn badges(
            &self,
            _tenant: &crate::tenancy::TenantContext,
            id: &str,
            author: &Principal,
        ) -> Result<Vec<&'static str>, Error> {
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
        async fn badges(
            &self,
            _tenant: &crate::tenancy::TenantContext,
            _: &str,
            _: &Principal,
        ) -> Result<Vec<&'static str>, Error> {
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
        let found = directory
            .badges(&crate::tenancy::test_tenant(), "catalog.product", "1", &manager)
            .await
            .unwrap();
        assert_eq!(found, ["seller", "verified_buyer"], "undeclared badges are dropped");
        assert_eq!(
            directory
                .badges(&crate::tenancy::test_tenant(), "catalog.product", "1", &buyer)
                .await
                .unwrap(),
            ["verified_buyer"]
        );
        assert!(
            directory
                .badges(&crate::tenancy::test_tenant(), "content.entry", "1", &manager)
                .await
                .unwrap()
                .is_empty()
        );
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

/// One history row of a record (step 51 P13): an event recorded about that record.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct HistoryLine {
    pub id: Uuid,
    pub at: Timestamp,
    /// Event name, e.g. `content.entry_published`.
    pub action: String,
    pub actor_user_id: Option<Uuid>,
    /// Name of the actor, if known.
    pub actor_name: Option<String>,
    /// Human-readable sentence of the event (`EventInfo::activity`), translated; `None` when the event has none.
    pub sentence: Option<String>,
}

impl HistoryLine {
    pub fn new(id: Uuid, at: Timestamp, action: impl Into<String>) -> Self {
        HistoryLine {
            id,
            at,
            action: action.into(),
            actor_user_id: None,
            actor_name: None,
            sentence: None,
        }
    }

    pub fn actor(mut self, user_id: Option<Uuid>, name: Option<String>) -> Self {
        self.actor_user_id = user_id;
        self.actor_name = name;
        self
    }

    pub fn sentence(mut self, sentence: Option<String>) -> Self {
        self.sentence = sentence;
        self
    }
}

/// History by record, published by the module that keeps the log (`audit`) with
/// `context.provide::<dyn RecordHistory>(...)`. The kernel has already checked that the caller can read the
/// record before asking.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait RecordHistory: Send + Sync {
    /// At most `limit` newest events with subject `subject` (`<kind>:<id>`), newest first.
    async fn history(
        &self,
        tenant: &TenantContext,
        subject: &str,
        limit: u32,
        language: crate::i18n::Language,
    ) -> Result<Vec<HistoryLine>, Error>;

    /// One page of the events with subject `subject`, ordered by event ID (UUIDv7, so the order they were
    /// recorded): newest first unless `page` asks `asc`; the cursor is the ID of the last line
    /// (D-20261125-list-pages).
    ///
    /// The default implementation exists only so that providers written before this method keep compiling: it
    /// reads every event through [`history`](Self::history) and cuts the page in memory, which costs as much as
    /// the whole history on every page. Providers should override it with a keyset page in their database (the
    /// `audit` module does).
    async fn history_page(
        &self,
        tenant: &TenantContext,
        subject: &str,
        page: &crate::pagination::PageRequest,
        language: crate::i18n::Language,
    ) -> Result<crate::pagination::Page<HistoryLine>, Error> {
        use crate::pagination::{Order, Page, Pagination};
        let mut lines = self.history(tenant, subject, u32::MAX, language).await?;
        let total = page.wants_total().then(|| u64::try_from(lines.len()).unwrap_or(u64::MAX));
        match page.order() {
            Order::Asc => lines.sort_by_key(|line| line.id),
            Order::Desc => lines.sort_by_key(|line| std::cmp::Reverse(line.id)),
        }
        if let Some(cursor) = page.cursor() {
            lines.retain(|line| match page.order() {
                Order::Asc => line.id > cursor,
                Order::Desc => line.id < cursor,
            });
        }
        let limit = page.limit() as usize;
        let has_more = lines.len() > limit;
        lines.truncate(limit);
        let next_cursor = has_more.then(|| lines.last().map(|line| line.id.to_string())).flatten();
        Ok(Page {
            data: lines,
            pagination: Pagination::new(page.limit(), next_cursor).with_total(total),
        })
    }
}
