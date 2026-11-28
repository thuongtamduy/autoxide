//! [`SearchIndex`]: the registry of searchable document kinds and the engine in use (step 40 P7). The kernel
//! publishes it at startup; modules get it with `context.require::<SearchIndex>()` or declare through
//! `StartContext::search_documents`.
//!
//! Sync: modules do NOT call the engine inside a write transaction (an external engine means a network call).
//! Inside the transaction of the change, the module calls [`SearchIndex::enqueue`], which queues a
//! `kernel.search_sync` job carrying the kind and IDs, in the same transaction (like the outbox). The job
//! re-reads the current document through [`SearchSource::load`]: if present, `index`; if gone (deleted),
//! `remove`. Running it any number of times gives the same result, so it tolerates redelivery (D-054).

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError, RwLock};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::backend::{
    OrgScope, SearchBackend, SearchHits, SearchKind, SearchRequest, SearchSource,
};
use super::database::DatabaseSearch;
use crate::access::{Principal, Scope};
use crate::config::Setting;
use crate::error::{Error, FieldError};
use crate::job::{EnqueueOptions, Job, JobContext, JobHandler, JobQueue};
use crate::tenancy::{DatabaseRegistry, TenantContext};

/// `AUTOXIDE_SEARCH_BACKEND`.
pub(crate) const SEARCH_BACKEND: Setting = Setting::new(
    "AUTOXIDE_SEARCH_BACKEND",
    "Search engine that keeps the search index: `database` (default: a table in each tenant's \
     database, nothing else to run) or the name of an engine a module adds (e.g. a Meilisearch \
     module). Changing it needs `autoxide task kernel.reindex` to fill the new engine.",
)
.with_default("database");

/// Name of the default engine.
pub const DATABASE_BACKEND: &str = "database";

/// Job kind for sync.
pub(crate) const SEARCH_SYNC: &str = "kernel.search_sync";

/// Documents per batch when rebuilding the index.
const BATCH: u64 = 200;
/// Maximum number of IDs in one sync job.
const MAX_SYNC_IDS: usize = 500;

/// The [`SEARCH_SYNC`] job: syncs the records `ids` of kind `kind`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SyncJob {
    pub kind: String,
    pub ids: Vec<Uuid>,
}

impl Job for SyncJob {
    const KIND: &'static str = SEARCH_SYNC;
}

/// A declared kind and its document source.
type Entry = (SearchKind, Arc<dyn SearchSource>);

/// Registry of document kinds (shared between [`SearchIndex`] and [`DatabaseSearch`]).
#[derive(Clone, Default)]
pub(crate) struct Kinds {
    entries: Arc<RwLock<BTreeMap<String, Entry>>>,
}

impl Kinds {
    pub(crate) fn kind(&self, name: &str) -> Result<SearchKind, Error> {
        let entries = self.entries.read().unwrap_or_else(PoisonError::into_inner);
        entries.get(name).map(|(kind, _)| kind.clone()).ok_or_else(|| unknown_kind(name))
    }

    fn source(&self, name: &str) -> Result<Entry, Error> {
        let entries = self.entries.read().unwrap_or_else(PoisonError::into_inner);
        entries.get(name).cloned().ok_or_else(|| unknown_kind(name))
    }

    fn names(&self) -> Vec<String> {
        self.entries.read().unwrap_or_else(PoisonError::into_inner).keys().cloned().collect()
    }
}

fn unknown_kind(name: &str) -> Error {
    Error::not_found(
        "kernel.search_kind_unknown",
        "No module declares the searchable kind `{kind}`.",
    )
    .with_param("kind", name)
}

/// Registry of document kinds and the search engine. Cheap to clone.
#[derive(Clone)]
pub struct SearchIndex {
    kinds: Kinds,
    configured: String,
    database: Arc<DatabaseSearch>,
    backends: Arc<RwLock<BTreeMap<String, Arc<dyn SearchBackend>>>>,
    jobs: JobQueue,
}

impl SearchIndex {
    /// An empty registry using the `configured` engine (`database` or the name of an engine a module will register).
    pub fn new(configured: &str) -> Self {
        let kinds = Kinds::default();
        SearchIndex {
            database: Arc::new(DatabaseSearch::new(kinds.clone())),
            kinds,
            configured: configured.to_owned(),
            backends: Arc::default(),
            jobs: JobQueue::new(crate::builtin::NAME),
        }
    }

    /// Declares the document kind `kind` of module `module` (`<module>.<kind>`) and where to read documents.
    pub fn register(
        &self,
        module: &str,
        kind: SearchKind,
        source: Arc<dyn SearchSource>,
    ) -> Result<(), Error> {
        if !kind.kind.starts_with(&format!("{module}.")) {
            return Err(Error::internal(format!(
                "searchable kind `{}` of module `{module}` must be named `{module}.<type>`",
                kind.kind
            )));
        }
        if kind.ranked_fields().is_empty() {
            return Err(Error::internal(format!(
                "searchable kind `{}` declares no field with a weight above 0",
                kind.kind
            )));
        }
        let mut entries = self.kinds.entries.write().unwrap_or_else(PoisonError::into_inner);
        if entries.contains_key(kind.kind) {
            return Err(Error::internal(format!(
                "searchable kind `{}` is declared twice",
                kind.kind
            )));
        }
        entries.insert(kind.kind.to_owned(), (kind, source));
        Ok(())
    }

    /// Registers the engine named `name` (an external engine module). Used when `AUTOXIDE_SEARCH_BACKEND = name`.
    pub fn add_backend(&self, name: &str, backend: Arc<dyn SearchBackend>) -> Result<(), Error> {
        if name == DATABASE_BACKEND {
            return Err(Error::internal("the search backend name `database` is the built-in one"));
        }
        let mut backends = self.backends.write().unwrap_or_else(PoisonError::into_inner);
        if backends.insert(name.to_owned(), backend).is_some() {
            return Err(Error::internal(format!("search backend `{name}` is registered twice")));
        }
        Ok(())
    }

    /// The engine in use.
    pub fn backend(&self) -> Result<Arc<dyn SearchBackend>, Error> {
        if self.configured == DATABASE_BACKEND {
            return Ok(self.database.clone());
        }
        let backends = self.backends.read().unwrap_or_else(PoisonError::into_inner);
        backends.get(&self.configured).cloned().ok_or_else(|| {
            Error::internal(format!(
                "AUTOXIDE_SEARCH_BACKEND is `{}` but no module adds a search backend of that name",
                self.configured
            ))
        })
    }

    /// Name of the configured engine.
    pub fn configured(&self) -> &str {
        &self.configured
    }

    /// Queues the sync of records `ids` of kind `kind`, on the caller's `connection` (the transaction of the
    /// change); the job runs after commit.
    pub async fn enqueue<C: crate::db::Executor>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        kind: &str,
        ids: &[Uuid],
    ) -> Result<(), Error> {
        let connection = &crate::db::Via::of(connection);
        self.kinds.kind(kind)?;
        for chunk in ids.chunks(MAX_SYNC_IDS) {
            let job = SyncJob { kind: kind.to_owned(), ids: chunk.to_vec() };
            self.jobs.enqueue(connection, tenant, &job, EnqueueOptions::default()).await?;
        }
        Ok(())
    }

    /// Syncs the records `ids` of kind `kind` right away (the job's work): a document that still exists is
    /// put into the index, one that no longer exists is removed.
    pub async fn sync(
        &self,
        tenant: &TenantContext,
        kind: &str,
        ids: &[Uuid],
    ) -> Result<(), Error> {
        let (_, source) = self.kinds.source(kind)?;
        let backend = self.backend()?;
        let docs = source.load(tenant, ids).await?;
        let docs: Vec<_> = docs.into_iter().filter(|doc| doc.kind == kind).collect();
        let gone: Vec<Uuid> =
            ids.iter().copied().filter(|id| !docs.iter().any(|doc| doc.id == *id)).collect();
        backend.index(tenant, &docs).await?;
        backend.remove(tenant, kind, &gone).await
    }

    /// Rebuilds the index of kind `kind` (every kind when `None`) of tenant `tenant`. Returns the document count.
    pub async fn rebuild(
        &self,
        tenant: &TenantContext,
        kind: Option<&str>,
    ) -> Result<usize, Error> {
        let names = match kind {
            Some(kind) => vec![self.kinds.kind(kind)?.kind.to_owned()],
            None => self.kinds.names(),
        };
        let backend = self.backend()?;
        let mut count = 0;
        for name in names {
            let (_, source) = self.kinds.source(&name)?;
            backend.clear(tenant, &name).await?;
            let mut after = None;
            loop {
                let docs = source.scan(tenant, after, BATCH).await?;
                let Some(last) = docs.last() else { break };
                after = Some(last.id);
                count += docs.len();
                backend.index(tenant, &docs).await?;
                if (docs.len() as u64) < BATCH {
                    break;
                }
            }
        }
        Ok(count)
    }

    /// Searches kind `request.kind` for the caller `principal`: checks the kind's permission and limits to
    /// its node scope (filtered inside the engine). `principal = None`: public kinds only.
    pub async fn search(
        &self,
        tenant: &TenantContext,
        principal: Option<&Principal>,
        mut request: SearchRequest,
    ) -> Result<SearchHits, Error> {
        let kind = self.kinds.kind(&request.kind)?;
        let unknown: Vec<FieldError> = request
            .filters
            .keys()
            .filter(|name| !kind.filters.contains(&name.as_str()))
            .map(|name| {
                FieldError::new("filters", "unknown", "`{name}` is not a filter of this kind")
                    .with_param("name", name)
            })
            .collect();
        if !unknown.is_empty() {
            return Err(Error::validation(unknown));
        }
        request.scope = match (&kind.permission, principal) {
            (None, _) => None,
            (Some(permission), Some(principal)) => scope_of(principal, permission)?,
            (Some(_), None) => {
                return Err(Error::unauthenticated(
                    "unauthenticated",
                    "Sign in to search this kind of record.",
                ));
            }
        };
        self.backend()?.query(tenant, &kind, &request).await
    }
}

/// Node scope of `permission` for `principal`: `None` when it has whole-tenant depth and no fence.
fn scope_of(
    principal: &Principal,
    permission: &crate::access::Permission,
) -> Result<Option<OrgScope>, Error> {
    let grants = principal.grants(permission);
    if grants.is_empty() {
        principal.require(permission)?;
    }
    let fence = principal.fence();
    if grants.iter().any(|grant| grant.scope == Scope::Tenant) && fence.is_none() {
        return Ok(None);
    }
    let mut scope = OrgScope { fence, ..OrgScope::default() };
    for grant in grants {
        match grant.scope {
            Scope::Tenant => scope.subtrees.push(crate::tenancy::root_of(principal.tenant_id())),
            Scope::Subtree => scope.subtrees.push(grant.org),
            // Tài liệu không có người sở hữu: "của mình" hiểu như "đúng nút" (như `scope_condition`).
            Scope::Unit | Scope::Own => scope.units.push(grant.org),
        }
    }
    Ok(Some(scope))
}

/// Runner of the [`SEARCH_SYNC`] job.
pub(crate) struct SyncRunner(pub SearchIndex);

#[async_trait::async_trait]
impl JobHandler for SyncRunner {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let tenant = job
            .tenant
            .as_ref()
            .ok_or_else(|| Error::internal("a search sync job must belong to a tenant"))?;
        let input: SyncJob =
            serde_json::from_value(job.payload.clone()).map_err(Error::internal)?;
        self.0.sync(tenant, &input.kind, &input.ids).await
    }
}

/// The command `autoxide task kernel.reindex [kind=<kind>] [tenant=<slug>]`.
pub(crate) fn reindex_task(index: SearchIndex, registry: DatabaseRegistry) -> crate::task::Task {
    crate::task::Task::new(
        "reindex",
        "Rebuild the search index from the records of the modules: every searchable kind, or only \
         `kind`; every tenant, or only `tenant` (its slug). Run it after changing \
         AUTOXIDE_SEARCH_BACKEND or when the index looks out of date.",
        Arc::new(Reindex { index, registry }),
    )
    .argument(crate::task::TaskArgument::optional(
        "kind",
        "Only this kind, e.g. `catalog.product`.",
    ))
    .argument(crate::task::TaskArgument::optional("tenant", "Only the tenant with this slug."))
}

struct Reindex {
    index: SearchIndex,
    registry: DatabaseRegistry,
}

#[async_trait::async_trait]
impl crate::task::TaskRun for Reindex {
    async fn run(&self, args: &crate::task::TaskArgs) -> Result<String, Error> {
        let kind = args.get("kind");
        let tenants = match args.get("tenant") {
            Some(slug) => vec![self.registry.tenant_by_slug(slug).await?],
            None => self.registry.all_tenants().await?,
        };
        let mut documents = 0;
        for tenant in &tenants {
            documents += self.index.rebuild(tenant, kind).await?;
        }
        Ok(format!(
            "Indexed {documents} document(s) for {} tenant(s) with the `{}` search backend.",
            tenants.len(),
            self.index.configured()
        ))
    }
}
