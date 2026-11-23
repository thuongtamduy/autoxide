//! Replaceable search engine (step 40 P7, D-220): a module declares its **searchable document kinds**
//! ([`SearchKind`] + [`SearchSource`]), and the kernel keeps the index through a [`SearchBackend`]. The default
//! is [`DatabaseSearch`](super::DatabaseSearch): the table `kernel_search_docs` in the tenant's own database,
//! where an exact-diacritic match ranks before a diacritic-insensitive one (D-202), behaving the same on
//! SQLite and Postgres. A dedicated search engine (Meilisearch, Typesense, OpenSearch…) is a separate module
//! that implements this trait and registers via `StartContext::search_backend`; it is selected with
//! `AUTOXIDE_SEARCH_BACKEND`, and business modules change nothing.
//!
//! This differs from the "contains" box on each module's search key column ([`fold`](super::fold),
//! [`contains`](super::contains)) and from cross-module search (`autoxide_kernel::find`): those two remain
//! unchanged (no API break); a module moves to the index when its table grows large enough that "contains"
//! scans too slowly.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::access::Permission;
use crate::error::Error;
use crate::pagination::{Page, PageRequest};
use crate::tenancy::{OrgId, TenantContext};

/// A document in the index: a snapshot of the searchable part of a record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchDoc {
    /// Kind, `<module>.<kind>` (like the object registry), e.g. `catalog.product`.
    pub kind: String,
    /// Record ID.
    pub id: Uuid,
    /// Node that owns the record (permission scope is filtered inside the backend).
    pub org_id: OrgId,
    /// Language of the content (`vi`, `en`…); empty: not language-specific.
    pub locale: String,
    /// Searchable fields, by the names declared in [`SearchKind::fields`].
    pub fields: BTreeMap<String, String>,
    /// Filter values, by the names declared in [`SearchKind::filters`] (string, number or bool).
    pub filters: BTreeMap<String, Value>,
}

impl SearchDoc {
    pub fn new(kind: impl Into<String>, id: Uuid, org_id: OrgId) -> Self {
        SearchDoc {
            kind: kind.into(),
            id,
            org_id,
            locale: String::new(),
            fields: BTreeMap::new(),
            filters: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn locale(mut self, locale: impl Into<String>) -> Self {
        self.locale = locale.into();
        self
    }

    #[must_use]
    pub fn field(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.fields.insert(name.into(), value.into());
        self
    }

    #[must_use]
    pub fn filter(mut self, name: impl Into<String>, value: impl Into<Value>) -> Self {
        self.filters.insert(name.into(), value.into());
        self
    }
}

/// Declaration of one searchable document kind.
#[derive(Debug, Clone)]
pub struct SearchKind {
    /// `<module>.<kind>`.
    pub kind: &'static str,
    /// Searchable fields and their weights (higher ranks first; the highest weight is the "title").
    pub fields: Vec<(&'static str, u8)>,
    /// Names of the equality filters.
    pub filters: Vec<&'static str>,
    /// Permission required to read results; its node scope limits the results. `None`: public.
    pub permission: Option<Permission>,
}

impl SearchKind {
    pub fn new(kind: &'static str) -> Self {
        SearchKind { kind, fields: Vec::new(), filters: Vec::new(), permission: None }
    }

    #[must_use]
    pub fn field(mut self, name: &'static str, weight: u8) -> Self {
        self.fields.push((name, weight));
        self
    }

    #[must_use]
    pub fn filter(mut self, name: &'static str) -> Self {
        self.filters.push(name);
        self
    }

    #[must_use]
    pub fn permission(mut self, permission: Permission) -> Self {
        self.permission = Some(permission);
        self
    }

    /// Fields by descending weight (same weight: declaration order).
    pub fn ranked_fields(&self) -> Vec<&'static str> {
        let mut fields = self.fields.clone();
        fields.sort_by_key(|(_, weight)| std::cmp::Reverse(*weight));
        fields.into_iter().filter(|(_, weight)| *weight > 0).map(|(name, _)| name).collect()
    }
}

/// Node scope of a query, taken from the caller's grants (like `Principal::scope_condition`).
/// The backend must filter RIGHT INSIDE the index, not afterwards (so result pages stay correct).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OrgScope {
    /// Records at these nodes or their child nodes.
    pub subtrees: Vec<OrgId>,
    /// Records at exactly these nodes.
    pub units: Vec<OrgId>,
    /// Fence: only within this subtree (in addition to the conditions above).
    pub fence: Option<OrgId>,
}

/// A query on one document kind.
#[derive(Debug, Clone)]
pub struct SearchRequest {
    pub kind: String,
    /// Text typed by the user; empty: every document (subject to the filters).
    pub text: String,
    /// Equality filters (declared name → value as a string).
    pub filters: BTreeMap<String, String>,
    pub locale: Option<String>,
    /// `None`: the whole tenant.
    pub scope: Option<OrgScope>,
    pub page: PageRequest,
}

/// One result.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchHit {
    pub kind: String,
    pub id: Uuid,
    pub org_id: OrgId,
    pub locale: String,
    pub fields: BTreeMap<String, String>,
}

/// A page of results (`limit` / `cursor` as in every list).
pub type SearchHits = Page<SearchHit>;

/// The engine that holds the index. Every function takes a tenant: documents of one tenant never surface in
/// another. Implementations must tolerate being called again (the sync job re-runs after a failure): `index`
/// is an upsert, and `remove` of a missing document succeeds.
#[async_trait::async_trait]
pub trait SearchBackend: Send + Sync {
    /// Adds or replaces documents.
    async fn index(&self, tenant: &TenantContext, docs: &[SearchDoc]) -> Result<(), Error>;
    /// Removes the documents `ids` of kind `kind`.
    async fn remove(&self, tenant: &TenantContext, kind: &str, ids: &[Uuid]) -> Result<(), Error>;
    /// Removes every document of kind `kind` (before rebuilding the index).
    async fn clear(&self, tenant: &TenantContext, kind: &str) -> Result<(), Error>;
    /// Searches. The query `kind` has already been checked to be a declared kind; filters only contain declared names.
    async fn query(
        &self,
        tenant: &TenantContext,
        kind: &SearchKind,
        request: &SearchRequest,
    ) -> Result<SearchHits, Error>;
}

/// Where a module supplies the documents of one kind (read from the module's own tables).
#[async_trait::async_trait]
pub trait SearchSource: Send + Sync {
    /// Current documents of the records `ids`; a deleted record is absent (and dropped from the index).
    async fn load(&self, tenant: &TenantContext, ids: &[Uuid]) -> Result<Vec<SearchDoc>, Error>;
    /// Up to `limit` documents with ID greater than `after`, in ascending ID order (index rebuild).
    async fn scan(
        &self,
        tenant: &TenantContext,
        after: Option<Uuid>,
        limit: u64,
    ) -> Result<Vec<SearchDoc>, Error>;
}
