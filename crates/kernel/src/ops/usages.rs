//! "Where is this used" (step 51, D-267): which records of which module point at a record (a post uses an
//! image, a menu points at a post…). A module declares a [`ReferenceSource`] — it reads its own tables itself
//! (architecture rule 1); the kernel gathers them through `GET /api/v1/kernel/records/{kind}/{id}/usages`,
//! returning only records of modules for which the caller has `<module>.read`.

use std::sync::{Arc, PoisonError, RwLock};

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::error::Error;
use crate::tenancy::TenantContext;

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
