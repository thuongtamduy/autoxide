//! Database registry (spec 33 "Connection registry"): the control database (`DATABASE_URL`, placement `main`)
//! and the other placements, opened lazily, at most `AUTOXIDE_TENANCY_MAX_OPEN_DATABASES` pools at once.
//!
//! - `single` mode: everything is the default tenant on the control database; no extra queries, so the hot
//!   path is as before step 33.
//! - `multi` mode: looks up a tenant by ID / slug / domain through control-plane tables, keeping results in a
//!   cache with a TTL (`AUTOXIDE_TENANCY_CACHE_SECONDS`); when a tenant is edited on this server,
//!   [`DatabaseRegistry::invalidate`] runs at once, and other servers see it after at most the TTL.
//! - The connection URL of a placement is stored encrypted (`Sealer`, purpose [`PLACEMENT_URL`], bound to the
//!   placement ID); the registry opens with the program root key, which the `kernel` module supplies at
//!   startup ([`DatabaseRegistry::use_secret_key`]).
//! - A tenant on a placement whose `schema_state` is not `ready` carries `schema_ready = false` and receives
//!   `tenant_maintenance` (spec 33); tenants elsewhere run normally.
//! - A placement with its own file storage (`storage_sealed`, encrypted, purpose [`PLACEMENT_STORAGE`]): the
//!   tenant context carries that storage ([`TenantContext::own_storage`]); the storage client is cached per
//!   placement and rebuilt when the stored value changes. The storage opener (`dyn StorageOpener`) and the
//!   shared storage are published by the `files` module ([`DatabaseRegistry::use_storage`]).
//! - The least recently used pool is dropped from the registry when room is needed; a request holding a clone
//!   of it keeps running to completion (the pool closes itself when no clone remains); never call `close()`
//!   here.

use crate::time::Timestamp;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Query, SelectStatement};
use sea_orm::{ConnectionTrait, QueryResult};
use uuid::Uuid;

use super::{
    DEFAULT_TENANT, MAIN_PLACEMENT, NO_TENANT, OrgId, PlacementId, TenancyConfig, TenancyMode,
    TenantContext, TenantId, TenantStatus, errors,
};
use crate::config::{DatabaseConfig, DatabaseUrl};
use crate::database::Database;
use crate::error::Error;
use crate::secret::SecretKey;
use crate::storage::{Storage, StorageLocation, StorageOpener, UnavailableStorage};

/// Purpose (AAD of `Sealer`) of the placement connection URL; the row is the placement ID.
pub const PLACEMENT_URL: &str = "kernel.placement_url";
/// Purpose of the read URL (read replica) of a placement.
pub const PLACEMENT_READ_URL: &str = "kernel.placement_read_url";
/// Purpose of the placement's own file storage (JSON of [`StorageLocation`]); the row is the placement ID.
pub const PLACEMENT_STORAGE: &str = "kernel.placement_storage";

/// A file storage and the tenants that use it (the orphan-file cleanup job scans each storage): the shared
/// storage (`placement = None`) or a placement's own storage.
#[derive(Clone)]
pub struct FileStore {
    pub placement: Option<PlacementId>,
    pub storage: Arc<dyn Storage>,
}

impl FileStore {
    /// Tenant `tenant` stores files here (keys of other tenants found here, e.g. copies left over after a
    /// move, are not the concern of this scan).
    pub fn serves(&self, tenant: &TenantContext) -> bool {
        tenant.storage_placement() == self.placement
    }
}

/// An opened own storage and the stored (encrypted) value used to build it: when the value in the table
/// changes (another server changed the storage), it is rebuilt.
type OpenStore = (String, Arc<dyn Storage>);

/// The shared storage and the opener for own storage (published by the `files` module; filled after every module starts).
struct StorageSlot {
    shared: Option<Arc<dyn Storage>>,
    opener: Option<Arc<dyn StorageOpener>>,
}

/// A descendant tenant (see [`DatabaseRegistry::tenant_descendants`]).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ChildTenant {
    pub id: TenantId,
    pub parent_id: TenantId,
    pub slug: String,
    pub name: String,
    /// `provisioning`, `active`, `suspended`, `maintenance` hay `moving`.
    pub status: String,
}

/// Database registry of the installation. Cheap to clone (it holds an `Arc` inside).
#[derive(Clone)]
pub struct DatabaseRegistry(Arc<Inner>);

struct Inner {
    mode: TenancyMode,
    config: TenancyConfig,
    control: Database,
    /// Configuration of the control database: the template used when opening other placements (timeouts…).
    base: Option<DatabaseConfig>,
    default: TenantContext,
    tenants: Mutex<HashMap<TenantId, (Instant, TenantContext)>>,
    slugs: Mutex<HashMap<String, (Instant, TenantId)>>,
    /// Domain → (tenant, node bound to the domain) verified; `None`: none.
    domains: Mutex<HashMap<String, (Instant, Option<DomainSite>)>>,
    pools: Mutex<Pools>,
    /// Root key used to open encrypted placement URLs; the `kernel` module sets it at startup.
    secret: OnceLock<SecretKey>,
    /// Shared storage and the opener for own storage; filled after every module starts.
    storage: OnceLock<StorageSlot>,
    /// Own storages already opened, by placement, with the stored (encrypted) value used to build each.
    stores: Mutex<HashMap<PlacementId, OpenStore>>,
    /// Only one party opens a placement pool at a time (if two requests need the same unopened placement,
    /// the later one uses the pool the earlier one just opened).
    opening: tokio::sync::Mutex<()>,
}

#[derive(Default)]
struct Pools {
    open: HashMap<PlacementId, (u64, Database)>,
    tick: u64,
}

impl std::fmt::Debug for DatabaseRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DatabaseRegistry").field("mode", &self.0.mode).finish_non_exhaustive()
    }
}

/// Tenant and node (website) of a verified domain.
type DomainSite = (TenantId, Option<OrgId>);

/// Locks `mutex` even if another thread panicked while holding it: the data inside is only a cache.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl DatabaseRegistry {
    /// Registry of `single` mode: every request is the default tenant on `control`.
    pub fn single(control: Database) -> Self {
        Self::build(control, None, TenancyConfig::default())
    }

    /// Registry according to `config`: `base` is the configuration of the control database
    /// (`DATABASE_URL`), used as the template when opening other placements.
    pub fn new(control: Database, base: DatabaseConfig, config: TenancyConfig) -> Self {
        Self::build(control, Some(base), config)
    }

    fn build(control: Database, base: Option<DatabaseConfig>, config: TenancyConfig) -> Self {
        DatabaseRegistry(Arc::new(Inner {
            mode: config.mode,
            default: TenantContext::default_on(control.clone()),
            config,
            control,
            base,
            tenants: Mutex::default(),
            slugs: Mutex::default(),
            domains: Mutex::default(),
            pools: Mutex::default(),
            secret: OnceLock::new(),
            storage: OnceLock::new(),
            stores: Mutex::default(),
            opening: tokio::sync::Mutex::new(()),
        }))
    }

    pub fn mode(&self) -> TenancyMode {
        self.0.mode
    }

    pub fn config(&self) -> &TenancyConfig {
        &self.0.config
    }

    /// The control database (placement `main`): the control-plane tables and the default tenant's data.
    pub fn control(&self) -> &Database {
        &self.0.control
    }

    /// The ancestor tenants of tenant `id` (the parent-child tenant chain, step 34): the parent first, up to
    /// the root tenant of the chain. Empty when there is no parent or no such tenant. Read at the control
    /// database, one statement per level (short chains); stops at 16 levels so corrupt data cannot loop
    /// forever.
    pub async fn tenant_ancestors(&self, id: TenantId) -> Result<Vec<TenantId>, Error> {
        use sea_orm::EntityTrait;
        let mut found = Vec::new();
        let mut current = id.as_uuid();
        for _ in 0..16 {
            let row = super::admin::tenant::Entity::find_by_id(current)
                .one(self.control().read())
                .await?;
            let Some(parent) = row.and_then(|row| row.parent_id) else { break };
            if parent == id.as_uuid() || found.contains(&TenantId::from_uuid(parent)) {
                break;
            }
            found.push(TenantId::from_uuid(parent));
            current = parent;
        }
        Ok(found)
    }

    /// Every descendant tenant of tenant `id` (child, grandchild…), level by level, one statement per level;
    /// stops at 16 levels like [`tenant_ancestors`](Self::tenant_ancestors). Returns `(id, parent tenant,
    /// slug, name, status)`.
    pub async fn tenant_descendants(&self, id: TenantId) -> Result<Vec<ChildTenant>, Error> {
        use super::admin::tenant;
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
        let mut found: Vec<ChildTenant> = Vec::new();
        let mut level = vec![id.as_uuid()];
        for _ in 0..16 {
            if level.is_empty() {
                break;
            }
            let rows = tenant::Entity::find()
                .filter(tenant::Column::ParentId.is_in(level.clone()))
                .order_by_asc(tenant::Column::Slug)
                .all(self.control().read())
                .await?;
            level = rows
                .iter()
                .map(|row| row.id)
                .filter(|child| {
                    *child != id.as_uuid() && !found.iter().any(|seen| seen.id.as_uuid() == *child)
                })
                .collect();
            for row in rows.into_iter().filter(|row| level.contains(&row.id)) {
                found.push(ChildTenant {
                    id: TenantId::from_uuid(row.id),
                    parent_id: row.parent_id.map(TenantId::from_uuid).unwrap_or(id),
                    slug: row.slug,
                    name: row.name,
                    status: row.status,
                });
            }
        }
        Ok(found)
    }

    /// The default tenant (the platform tenant) on the control database.
    pub fn default_tenant(&self) -> TenantContext {
        self.0.default.clone()
    }

    fn ttl(&self) -> Duration {
        Duration::from_secs(self.0.config.cache_seconds)
    }

    /// The context of tenant `id`. Missing: `tenant_not_found`.
    pub async fn tenant(&self, id: TenantId) -> Result<TenantContext, Error> {
        if self.0.mode == TenancyMode::Single {
            return if id == DEFAULT_TENANT {
                Ok(self.default_tenant())
            } else {
                Err(errors::tenant_not_found(&id.to_string()))
            };
        }
        // Đọc cache ở một câu riêng: khoá được trả ngay, không giữ qua `await` bên dưới.
        let cached = lock(&self.0.tenants).get(&id).cloned();
        if let Some((at, tenant)) = cached
            && at.elapsed() < self.ttl()
        {
            return Ok(tenant);
        }
        let select = tenant_select().and_where(tenant_column("id").eq(id.as_uuid())).to_owned();
        let row = self.0.control.read().query_one(&select).await?;
        let row = row.ok_or_else(|| errors::tenant_not_found(&id.to_string()))?;
        self.context_of(&row).await
    }

    /// Every tenant that still has data (in single-tenant mode: the default tenant), for commands that walk
    /// every tenant (e.g. rebuilding the search index). A tenant that cannot be opened (a broken placement…)
    /// is skipped, with a log entry.
    pub async fn all_tenants(&self) -> Result<Vec<TenantContext>, Error> {
        if self.0.mode == TenancyMode::Single {
            return Ok(vec![self.default_tenant()]);
        }
        let select = tenant_select()
            .and_where(tenant_column("status").is_not_in(["deleted", "merged"]))
            .to_owned();
        let rows = self.0.control.read().query_all(&select).await?;
        let mut found = Vec::with_capacity(rows.len());
        for row in &rows {
            match self.context_of(row).await {
                Ok(tenant) => found.push(tenant),
                Err(error) => tracing::warn!(%error, "a tenant was skipped"),
            }
        }
        Ok(found)
    }

    /// The context of the tenant with short name `slug`.
    pub async fn tenant_by_slug(&self, slug: &str) -> Result<TenantContext, Error> {
        if self.0.mode == TenancyMode::Single {
            return if slug == self.0.default.slug() {
                Ok(self.default_tenant())
            } else {
                Err(errors::tenant_not_found(slug))
            };
        }
        let cached = lock(&self.0.slugs).get(slug).cloned();
        if let Some((at, id)) = cached
            && at.elapsed() < self.ttl()
        {
            return self.tenant(id).await;
        }
        let select = tenant_select().and_where(tenant_column("slug").eq(slug)).to_owned();
        let row = self.0.control.read().query_one(&select).await?;
        let row = row.ok_or_else(|| errors::tenant_not_found(slug))?;
        self.context_of(&row).await
    }

    /// The tenant that owns the verified domain `host`, if any. `single` mode: always `None` (does not read
    /// domains).
    pub async fn tenant_by_domain(&self, host: &str) -> Result<Option<TenantContext>, Error> {
        match self.domain_site(host).await? {
            Some((id, _)) => self.tenant(id).await.map(Some),
            None => Ok(None),
        }
    }

    /// The tenant and node (website) of the verified domain `host` (step 34): a domain bound to a node is the
    /// website of that node; unbound → `None` (the website of the whole tenant). `single` mode: always `None`.
    pub async fn domain_site(
        &self,
        host: &str,
    ) -> Result<Option<(TenantId, Option<OrgId>)>, Error> {
        if self.0.mode == TenancyMode::Single {
            return Ok(None);
        }
        let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
        let cached = lock(&self.0.domains).get(&host).cloned();
        if let Some((at, found)) = cached
            && at.elapsed() < self.ttl()
        {
            return Ok(found);
        }
        let select = Query::select()
            .column(Alias::new("tenant_id"))
            .column(Alias::new("org_id"))
            .from(Alias::new("kernel_tenant_domains"))
            .and_where(Expr::col(Alias::new("domain")).eq(host.as_str()))
            .and_where(Expr::col(Alias::new("status")).eq("verified"))
            .to_owned();
        let found = match self.0.control.read().query_one(&select).await? {
            Some(row) => Some((
                TenantId::from_uuid(row.try_get::<Uuid>("", "tenant_id")?),
                row.try_get::<Option<Uuid>>("", "org_id")?.map(OrgId::from_uuid),
            )),
            None => None,
        };
        lock(&self.0.domains).insert(host, (Instant::now(), found));
        Ok(found)
    }

    /// The root key used to open encrypted placement URLs. Set once (the `kernel` module sets it at startup);
    /// later calls are ignored.
    pub fn use_secret_key(&self, key: SecretKey) {
        let _ = self.0.secret.set(key);
    }

    /// The shared storage (`shared`, of the `files` module) and the opener for a placement's own storage
    /// (`opener`). Set once (`Modules::start_in` sets it after every module starts); later calls are ignored.
    pub fn use_storage(
        &self,
        shared: Option<Arc<dyn Storage>>,
        opener: Option<Arc<dyn StorageOpener>>,
    ) {
        let _ = self.0.storage.set(StorageSlot { shared, opener });
    }

    /// The shared file storage (configuration of the `files` module), if a module publishes it.
    pub fn shared_storage(&self) -> Option<Arc<dyn Storage>> {
        self.0.storage.get().and_then(|slot| slot.shared.clone())
    }

    /// Opens the value `sealed` (the `storage_sealed` column of placement `id`).
    pub async fn storage_location(
        &self,
        id: PlacementId,
        sealed: &str,
    ) -> Result<StorageLocation, Error> {
        let plain =
            self.sealer().await?.open(NO_TENANT, PLACEMENT_STORAGE, &id.to_string(), sealed)?;
        serde_json::from_slice(&plain).map_err(|_| {
            Error::internal(format!(
                "placement {id}: the stored file storage settings are not valid"
            ))
        })
    }

    /// The own file storage of placement `id` (re-reads the table); `None`: the placement uses the shared storage.
    pub async fn placement_storage(
        &self,
        id: PlacementId,
    ) -> Result<Option<Arc<dyn Storage>>, Error> {
        if id == MAIN_PLACEMENT {
            return Ok(None);
        }
        let select = Query::select()
            .column(Alias::new("storage_sealed"))
            .from(Alias::new("kernel_placements"))
            .and_where(Expr::col(Alias::new("id")).eq(id.as_uuid()))
            .to_owned();
        let row = self.0.control.read().query_one(&select).await?;
        let row = row.ok_or_else(|| errors::tenant_placement_not_found(&id.to_string()))?;
        let sealed: Option<String> = row.try_get("", "storage_sealed")?;
        match sealed {
            Some(sealed) => self.open_storage(id, &sealed).await.map(Some),
            None => Ok(None),
        }
    }

    /// The own storage built from `sealed` of placement `id`, through the cache.
    async fn open_storage(&self, id: PlacementId, sealed: &str) -> Result<Arc<dyn Storage>, Error> {
        let cached = lock(&self.0.stores).get(&id).cloned();
        if let Some((known, store)) = cached
            && known == sealed
        {
            return Ok(store);
        }
        let location = self.storage_location(id, sealed).await?;
        let opener =
            self.0.storage.get().and_then(|slot| slot.opener.clone()).ok_or_else(|| {
                Error::internal(format!(
                    "placement {id} has its own file storage, but no module opens file storage \
                 (register the `files` module)"
                ))
            })?;
        let store = opener.open(&location)?;
        lock(&self.0.stores).insert(id, (sealed.to_owned(), store.clone()));
        Ok(store)
    }

    /// Every file storage: `shared` (the shared storage), then each placement's own storage, by name.
    /// An own storage that cannot be opened is skipped (logged); the next round retries.
    pub async fn file_stores(&self, shared: Arc<dyn Storage>) -> Result<Vec<FileStore>, Error> {
        let mut all = vec![FileStore { placement: None, storage: shared }];
        if self.0.mode == TenancyMode::Single {
            return Ok(all);
        }
        let select = Query::select()
            .columns(["id", "storage_sealed"].map(Alias::new))
            .from(Alias::new("kernel_placements"))
            .and_where(Expr::col(Alias::new("id")).ne(MAIN_PLACEMENT.as_uuid()))
            .and_where(Expr::col(Alias::new("storage_sealed")).is_not_null())
            .order_by(Alias::new("name"), sea_orm::sea_query::Order::Asc)
            .to_owned();
        for row in self.0.control.read().query_all(&select).await? {
            let id = PlacementId::from_uuid(row.try_get::<Uuid>("", "id")?);
            let sealed: String = row.try_get("", "storage_sealed")?;
            match self.open_storage(id, &sealed).await {
                Ok(storage) => all.push(FileStore { placement: Some(id), storage }),
                Err(error) => {
                    tracing::warn!(placement_id = %id, error = %error, "cannot open the placement's file storage");
                }
            }
        }
        Ok(all)
    }

    /// Drops every tenant on placement `id` from the cache (after its `schema_state` or file storage changes).
    pub fn invalidate_placement(&self, id: PlacementId) {
        lock(&self.0.stores).remove(&id);
        let gone: Vec<TenantId> = lock(&self.0.tenants)
            .iter()
            .filter(|(_, (_, tenant))| tenant.placement() == id)
            .map(|(tenant, _)| *tenant)
            .collect();
        for tenant in gone {
            self.invalidate(tenant);
        }
    }

    /// Drops the domain `domain` from the cache (after verifying it, revoking it, or changing its tenant on
    /// this server): the next request re-reads the table.
    pub fn invalidate_domain(&self, domain: &str) {
        let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
        lock(&self.0.domains).remove(&domain);
    }

    /// Drops tenant `id` from the cache (after editing the tenant on this server).
    pub fn invalidate(&self, id: TenantId) {
        lock(&self.0.tenants).remove(&id);
        lock(&self.0.slugs).retain(|_, (_, known)| *known != id);
        lock(&self.0.domains).retain(|_, (_, known)| known.map(|(tenant, _)| tenant) != Some(id));
    }

    /// Every database this server runs jobs and events for: `(placement, name, database)`; only placements
    /// with all migrations applied. `single` mode: only the control database.
    pub async fn databases(&self) -> Result<Vec<(PlacementId, String, Database)>, Error> {
        let mut all = Vec::new();
        if self.0.config.worker_placements.includes("main") {
            all.push((MAIN_PLACEMENT, "main".to_owned(), self.0.control.clone()));
        }
        if self.0.mode == TenancyMode::Single {
            return Ok(all);
        }
        // Placement chưa đủ migration: job và event của nó đợi tới khi `migrate up --all` xong
        // (khách trên đó cũng đang nhận `tenant_maintenance`).
        let select = Query::select()
            .columns(["id", "name"].map(Alias::new))
            .from(Alias::new("kernel_placements"))
            .and_where(Expr::col(Alias::new("id")).ne(MAIN_PLACEMENT.as_uuid()))
            .and_where(Expr::col(Alias::new("schema_state")).eq("ready"))
            .order_by(Alias::new("name"), sea_orm::sea_query::Order::Asc)
            .to_owned();
        for row in self.0.control.read().query_all(&select).await? {
            let name: String = row.try_get("", "name")?;
            if !self.0.config.worker_placements.includes(&name) {
                continue;
            }
            let id = PlacementId::from_uuid(row.try_get::<Uuid>("", "id")?);
            all.push((id, name, self.placement_database(id).await?));
        }
        Ok(all)
    }

    /// The database of placement `id`; opens the pool if not yet open.
    pub async fn placement_database(&self, id: PlacementId) -> Result<Database, Error> {
        if id == MAIN_PLACEMENT {
            return Ok(self.0.control.clone());
        }
        if let Some(database) = self.touch(id) {
            return Ok(database);
        }
        let _opening = self.0.opening.lock().await;
        if let Some(database) = self.touch(id) {
            return Ok(database);
        }
        let select = Query::select()
            .columns(["url_sealed", "read_url_sealed"].map(Alias::new))
            .from(Alias::new("kernel_placements"))
            .and_where(Expr::col(Alias::new("id")).eq(id.as_uuid()))
            .to_owned();
        let row = self.0.control.read().query_one(&select).await?;
        let row = row.ok_or_else(|| errors::tenant_placement_not_found(&id.to_string()))?;
        let url: Option<String> = row.try_get("", "url_sealed")?;
        let read_url: Option<String> = row.try_get("", "read_url_sealed")?;
        let Some(url) = url else {
            // Không có URL riêng: cùng database điều khiển.
            return Ok(self.0.control.clone());
        };
        let url = self.unseal(id, PLACEMENT_URL, &url).await?;
        let read_url = match read_url {
            Some(sealed) => Some(self.unseal(id, PLACEMENT_READ_URL, &sealed).await?),
            None => None,
        };
        let database = self.open(&url, read_url.as_deref()).await?;
        let mut pools = lock(&self.0.pools);
        pools.tick += 1;
        let tick = pools.tick;
        pools.open.insert(id, (tick, database.clone()));
        while pools.open.len() > self.0.config.max_open_databases {
            // Bỏ pool ít dùng nhất khỏi sổ; bản clone đang dùng vẫn chạy tới xong.
            let oldest = pools.open.iter().min_by_key(|(_, (used, _))| *used).map(|(id, _)| *id);
            match oldest {
                Some(oldest) => {
                    pools.open.remove(&oldest);
                }
                None => break,
            }
        }
        Ok(database)
    }

    /// Number of placement pools currently open (the control database not counted).
    pub fn open_databases(&self) -> usize {
        lock(&self.0.pools).open.len()
    }

    fn touch(&self, id: PlacementId) -> Option<Database> {
        let mut pools = lock(&self.0.pools);
        pools.tick += 1;
        let tick = pools.tick;
        pools.open.get_mut(&id).map(|(used, database)| {
            *used = tick;
            database.clone()
        })
    }

    /// Opens an encrypted placement URL. An unencrypted value (copied into the table by hand) is returned verbatim.
    async fn unseal(&self, id: PlacementId, purpose: &str, sealed: &str) -> Result<String, Error> {
        if !crate::secret::is_sealed(sealed) {
            return Ok(sealed.to_owned());
        }
        let plain = self.sealer().await?.open(NO_TENANT, purpose, &id.to_string(), sealed)?;
        String::from_utf8(plain.to_vec())
            .map_err(|_| Error::internal(format!("placement {id}: the stored URL is not text")))
    }

    /// The sealer of the root key (encrypts / opens placement URLs).
    pub(super) async fn sealer(&self) -> Result<crate::secret::Sealer, Error> {
        let key = self.0.secret.get().ok_or_else(|| {
            Error::internal("the database registry has no secret key to open placement URLs")
        })?;
        key.sealer().await
    }

    /// Opens a pool for one placement (the small pool size of `AUTOXIDE_TENANCY_*_CONNECTIONS`).
    pub(super) async fn open(&self, url: &str, read_url: Option<&str>) -> Result<Database, Error> {
        let parse = |raw: &str| {
            raw.parse::<DatabaseUrl>()
                .map_err(|error| Error::internal(format!("placement URL is invalid: {error}")))
        };
        let wait = self.0.base.as_ref().map_or(Duration::from_secs(5), |base| base.wait);
        let config = DatabaseConfig {
            url: parse(url)?,
            read_url: read_url.map(parse).transpose()?,
            write_connections: self.0.config.write_connections,
            read_connections: self.0.config.read_connections,
            wait,
        };
        Database::connect(&config).await.map_err(|error| Error::internal(error.to_string()))
    }

    async fn context_of(&self, row: &QueryResult) -> Result<TenantContext, Error> {
        let id = TenantId::from_uuid(row.try_get::<Uuid>("", "id")?);
        let slug: String = row.try_get("", "slug")?;
        let name: String = row.try_get("", "name")?;
        let status: String = row.try_get("", "status")?;
        let status = TenantStatus::parse(&status).ok_or_else(|| {
            Error::internal(format!("tenant {id} has an unknown status `{status}`"))
        })?;
        let placement = PlacementId::from_uuid(row.try_get::<Uuid>("", "placement_id")?);
        let schema_state: Option<String> = row.try_get("", "schema_state")?;
        // Database điều khiển luôn đủ migration (`serve` đòi trước khi khởi động).
        let ready = placement == MAIN_PLACEMENT || schema_state.as_deref() == Some("ready");
        let storage_sealed: Option<String> = row.try_get("", "storage_sealed")?;
        let read_only: bool = row.try_get("", "read_only")?;
        let sandbox_of: Option<Uuid> = row.try_get("", "sandbox_of")?;
        let sandbox_expires_at: Option<Timestamp> = row.try_get("", "sandbox_expires_at")?;
        let database = self.placement_database(placement).await?;
        let mut tenant = TenantContext::new(id, slug.clone(), name, status, placement, database)
            .with_schema_ready(ready)
            .with_read_only(read_only);
        if let Some(of) = sandbox_of {
            tenant = tenant.with_sandbox_of(TenantId::from_uuid(of), sandbox_expires_at);
        }
        if let Some(sealed) = storage_sealed
            && placement != MAIN_PLACEMENT
        {
            // Nơi lưu riêng hỏng không làm cả khách ngừng phục vụ: chỉ thao tác file báo lỗi.
            let store = match self.open_storage(placement, &sealed).await {
                Ok(store) => store,
                Err(error) => {
                    tracing::warn!(placement_id = %placement, error = %error, "cannot open the placement's file storage");
                    let unavailable: Arc<dyn Storage> =
                        Arc::new(UnavailableStorage(error.to_string()));
                    unavailable
                }
            };
            tenant = tenant.with_storage(store);
        }
        let now = Instant::now();
        lock(&self.0.tenants).insert(id, (now, tenant.clone()));
        lock(&self.0.slugs).insert(slug, (now, id));
        Ok(tenant)
    }
}

/// Column `column` of `kernel_tenants` (the table is named explicitly: the query also joins `kernel_placements`).
fn tenant_column(column: &'static str) -> Expr {
    Expr::col((Alias::new("kernel_tenants"), Alias::new(column)))
}

/// A tenant together with `schema_state` and the own file storage of the placement that holds it.
fn tenant_select() -> SelectStatement {
    let tenants = Alias::new("kernel_tenants");
    let placements = Alias::new("kernel_placements");
    Query::select()
        .columns(
            [
                "id",
                "slug",
                "name",
                "status",
                "placement_id",
                "read_only",
                "sandbox_of",
                "sandbox_expires_at",
            ]
            .map(|column| (tenants.clone(), Alias::new(column))),
        )
        .column((placements.clone(), Alias::new("schema_state")))
        .column((placements.clone(), Alias::new("storage_sealed")))
        .from(tenants.clone())
        .left_join(
            placements.clone(),
            Expr::col((placements, Alias::new("id"))).equals((tenants, Alias::new("placement_id"))),
        )
        .to_owned()
}
