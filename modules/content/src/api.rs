//! Contract of the `content` module for other modules.
//!
//! Currently only [`LegacyMenus`]: reads the old `content` menus (step 17) so the `menus` module can import
//! them (D-238). The `content` menus are deprecated, sunset 2027-05-14; after that date this trait is removed
//! along with the old menu endpoints. Obtain it with `context.require::<dyn LegacyMenus>()` in
//! `Module::start` (register AFTER `content`). This file does not reference the module's internals.

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::time::Timestamp;
use uuid::Uuid;

/// An old menu: key, language and tree of items (already ordered).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyMenu {
    pub key: String,
    pub locale: String,
    pub items: Vec<LegacyMenuItem>,
}

/// An item of an old menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyMenuItem {
    pub label: String,
    pub target: LegacyTarget,
    pub children: Vec<LegacyMenuItem>,
}

/// Target of an old menu item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyTarget {
    /// A post or page (kind `content.entry` of the object ledger).
    Entry(Uuid),
    /// A category or tag (kind `content.term`).
    Term(Uuid),
    /// An address: `https://`, `http://`, `mailto:` or a `/…` path.
    Url(String),
}

/// Reads the old `content` menus.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait LegacyMenus: Send + Sync {
    /// Every old menu of tenant `tenant`, by key then language.
    async fn legacy_menus(&self, tenant: &TenantContext) -> Result<Vec<LegacyMenu>, Error>;
}

/// Snapshot of the published version of a post / page (step 37 C1): enough for another module (e.g. the legal
/// documents of `compliance`) to copy it into an immutable version without reading `content`'s tables.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PublishedSnapshot {
    pub entry_id: Uuid,
    pub title: String,
    pub locale: String,
    /// The published ProseMirror JSON body.
    pub body: serde_json::Value,
    pub plain_text: String,
    /// Revision of the published version (not the draft).
    pub revision_id: Uuid,
    pub published_at: Timestamp,
}

/// Reads the published version of a post / page. Obtain it with `context.require::<dyn PublishedEntries>()`
/// (register AFTER `content`).
#[async_trait::async_trait]
pub trait PublishedEntries: Send + Sync {
    /// Published version of entry `entry_id`; not published or missing → `None`.
    async fn published_snapshot(
        &self,
        tenant: &TenantContext,
        entry_id: Uuid,
    ) -> Result<Option<PublishedSnapshot>, Error>;
}

/// Part of another module in the website shell (`GET /api/v1/content/public/site`, step 67, D-234): e.g. the
/// cookie consent settings and tracking codes of `compliance`. `content` registers before those modules so
/// it cannot `require` them; the later module plugs itself in through [`SiteExtensions`] (obtained with
/// `context.require::<dyn SiteExtensions>()` in `Module::start`). The result is in
/// `extensions.<module>` of the response, along with `ETag` and `max-age`. Returns public data only.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait SiteExtension: Send + Sync {
    /// The module's part for website `request.site` in language `request.locale`; `None`: nothing. Called
    /// when the website shell is recomputed (at most every 30 seconds per website), not on every request.
    async fn site(
        &self,
        request: &autoxide_kernel::blocks::BlockRequest<'_>,
    ) -> Result<Option<serde_json::Value>, Error>;
}

/// Registry of extensions to the website shell.
pub trait SiteExtensions: Send + Sync {
    /// Plugs `module`'s part (module name, once per module) into the website shell.
    fn register(
        &self,
        module: &'static str,
        extension: std::sync::Arc<dyn SiteExtension>,
    ) -> Result<(), Error>;
}
