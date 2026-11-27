//! "Where is this used" (step 51, D-267): which records of which module point at a record (a post uses an
//! image, a menu points at a post…). A module declares a [`ReferenceSource`] — it reads its own tables itself
//! (architecture rule 1); the kernel gathers them through `GET /api/v1/kernel/records/{kind}/{id}/usages`,
//! returning only records of modules for which the caller has `<module>.read`.

use std::sync::{Arc, PoisonError, RwLock};

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use uuid::Uuid;

use crate::error::Error;
use crate::pagination::{Order, Page, PageRequest, Pagination};
use crate::tenancy::TenantContext;

/// How many referrers the default [`ReferenceSource::referrers_page`] reads through
/// [`ReferenceSource::referrers`].
pub const REFERRERS_PAGE_FALLBACK_LIMIT: usize = 10_000;

/// A record that points to the record asked about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Referrer {
    /// Type of the pointing record, e.g. `content.entry`.
    #[schema(example = "content.entry")]
    pub kind: String,
    pub id: String,
    /// What staff call it (title, name).
    pub title: String,
    /// How it points: e.g. `featured_media`, `body`, `menu_item`.
    #[schema(example = "featured_media")]
    pub field: String,
}

impl Referrer {
    pub fn new(
        kind: impl Into<String>,
        id: impl Into<String>,
        title: impl Into<String>,
        field: impl Into<String>,
    ) -> Self {
        Referrer { kind: kind.into(), id: id.into(), title: title.into(), field: field.into() }
    }
}

/// A module's reference source.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait ReferenceSource: Send + Sync {
    /// At most `limit` records of the module that point at record `kind` / `id` of tenant `tenant`.
    /// Returns empty for a kind that cannot be pointed at.
    async fn referrers(
        &self,
        tenant: &TenantContext,
        kind: &str,
        id: &str,
        limit: usize,
    ) -> Result<Vec<Referrer>, Error>;

    /// One page of the records of the module that point at record `kind` / `id` of tenant `tenant`, each with
    /// the key that orders it: the ID of the pointing row (UUIDv7, so creation order), newest first unless
    /// `page` asks `asc`. The kernel reads every source with the same `page` (the same cursor key) and merges
    /// them by key, so the key must be unique within the source and the page must hold the first
    /// `page.limit()` rows after `page.cursor()`, with a `next_cursor` when more follow
    /// (D-20261125-list-pages). Returns an empty page for a kind that cannot be pointed at.
    ///
    /// The default implementation exists only so that sources written before this method keep compiling: it
    /// reads at most [`REFERRERS_PAGE_FALLBACK_LIMIT`] referrers through [`referrers`](Self::referrers), keys
    /// each one by a hash of its fields (stable, but **not** creation order, so the "newest first" promise of
    /// this method and of the usages endpoint does not hold for such a source) and cuts the page in memory — every
    /// page costs as much as that whole read, referrers past that limit are not reachable, and
    /// `pagination.total` is left out when the read reaches it. Sources should override it with a keyset page
    /// in their database.
    async fn referrers_page(
        &self,
        tenant: &TenantContext,
        kind: &str,
        id: &str,
        page: &PageRequest,
    ) -> Result<Page<KeyedReferrer>, Error> {
        let read = self.referrers(tenant, kind, id, REFERRERS_PAGE_FALLBACK_LIMIT).await?;
        let complete = read.len() < REFERRERS_PAGE_FALLBACK_LIMIT;
        let mut rows: Vec<KeyedReferrer> = read
            .into_iter()
            .map(|referrer| KeyedReferrer { key: hashed_key(&referrer), referrer })
            .collect();
        // Hai dòng giống hệt nhau cho cùng khoá: giữ một (bản gộp theo khoá không phân biệt được).
        rows.sort_by_key(|row| row.key);
        rows.dedup_by_key(|row| row.key);
        let total =
            (page.wants_total() && complete).then(|| u64::try_from(rows.len()).unwrap_or(u64::MAX));
        if page.order() == Order::Desc {
            rows.reverse();
        }
        if let Some(cursor) = page.cursor() {
            rows.retain(|row| match page.order() {
                Order::Asc => row.key > cursor,
                Order::Desc => row.key < cursor,
            });
        }
        let limit = page.limit() as usize;
        let has_more = rows.len() > limit;
        rows.truncate(limit);
        let next_cursor = has_more.then(|| rows.last().map(|row| row.key.to_string())).flatten();
        Ok(Page {
            data: rows,
            pagination: Pagination::new(page.limit(), next_cursor).with_total(total),
        })
    }
}

/// A [`Referrer`] with the key that orders it in a paged list: see [`ReferenceSource::referrers_page`].
/// Build it with [`KeyedReferrer::new`]: it is `#[non_exhaustive]` so fields can be added later without
/// breaking sources written outside this repository.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct KeyedReferrer {
    /// Unique within its source; the ID of the pointing row when the source has one.
    pub key: Uuid,
    pub referrer: Referrer,
}

impl KeyedReferrer {
    /// `referrer` ordered by `key` (unique within its source; the ID of the pointing row when it has one).
    pub fn new(key: Uuid, referrer: Referrer) -> Self {
        KeyedReferrer { key, referrer }
    }
}

/// Key of the default [`ReferenceSource::referrers_page`]: the first 16 bytes of a SHA-256 of the fields.
fn hashed_key(referrer: &Referrer) -> Uuid {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    for part in [&referrer.kind, &referrer.id, &referrer.title, &referrer.field] {
        hash.update(part.as_bytes());
        // Ký tự ngăn: ("ab","c") và ("a","bc") không ra cùng khoá.
        hash.update([0x1f]);
    }
    let digest = hash.finalize();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    Uuid::from_bytes(bytes)
}

type Entry = (&'static str, Arc<dyn ReferenceSource>);

/// Registry of every module's reference sources. Cheap to clone.
#[derive(Clone, Default)]
pub struct ReferenceSources {
    entries: Arc<RwLock<Vec<Entry>>>,
}

impl ReferenceSources {
    pub(crate) fn register(&self, module: &'static str, source: Arc<dyn ReferenceSource>) {
        self.entries.write().unwrap_or_else(PoisonError::into_inner).push((module, source));
    }

    pub(crate) fn all(&self) -> Vec<Entry> {
        self.entries.read().unwrap_or_else(PoisonError::into_inner).clone()
    }
}
