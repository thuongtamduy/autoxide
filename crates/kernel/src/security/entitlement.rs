//! Per-tenant entitlements (step 35, D-323): modules declare **features** ([`Feature`]) and **limits**
//! ([`Limit`]); one module (`plans`) publishes [`Entitlements`], the current entitlements of each tenant (plan,
//! add-on plans, overrides); every module asks through [`EntitlementsHandle`].
//!
//! The model is borrowed from Stripe Entitlements: the application only asks "what is this tenant allowed to
//! use", and does not infer it from invoices.
//!
//! - **No provider** (`plans` not installed), or a tenant with **no subscription and no default plan**: no
//!   limits at all, so the single-tenant model (mode `single`) is unchanged.
//! - The **platform tenant** (the operator's tenant) is always unlimited: operators do not lock themselves out.
//! - A module that is [`always_enabled`](crate::module::Module::always_enabled) (foundation: `auth`, `files`,
//!   ...) is never blocked by a plan.
//!
//! The handle exists before the provider starts (a module registered before `plans` can still hold it): the
//! kernel attaches the provider after every module has started.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, OnceLock};

use crate::error::Error;
use crate::tenancy::{DEFAULT_TENANT, TenantContext, TenantId};

/// A feature switched on / off by plan, key `<module>.<name>` (e.g. `content.ai_suggestions`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Feature {
    key: &'static str,
    description: &'static str,
}

impl Feature {
    /// Feature `key` (`<module>.<name>`), described in English for the operator who composes plans.
    pub const fn new(key: &'static str, description: &'static str) -> Self {
        Feature { key, description }
    }

    pub const fn key(&self) -> &'static str {
        self.key
    }

    pub const fn description(&self) -> &'static str {
        self.description
    }
}

/// Usage period of a [`LimitKind::Metered`] limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Period {
    /// Calendar month in UTC.
    Month,
}

/// Kind of limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LimitKind {
    /// Number of existing records (organization nodes, staff, products).
    Count,
    /// Amount currently in use, in `unit` (e.g. `bytes`).
    Amount { unit: &'static str },
    /// Usage within a period, resetting to 0 at the start of the period (requests, AI tokens).
    Metered { period: Period },
}

impl LimitKind {
    /// Kind name for the API: `count`, `amount`, `metered`.
    pub const fn as_str(&self) -> &'static str {
        match self {
            LimitKind::Count => "count",
            LimitKind::Amount { .. } => "amount",
            LimitKind::Metered { .. } => "metered",
        }
    }
}

/// A limit by plan, key `<module>.<name>` (e.g. `files.storage_bytes`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Limit {
    key: &'static str,
    description: &'static str,
    kind: LimitKind,
    parts: Option<&'static str>,
}

impl Limit {
    /// A limit that counts existing records.
    pub const fn count(key: &'static str, description: &'static str) -> Self {
        Limit { key, description, kind: LimitKind::Count, parts: None }
    }

    /// A limit on the amount currently in use, in unit `unit` (e.g. `bytes`).
    pub const fn amount(key: &'static str, description: &'static str, unit: &'static str) -> Self {
        Limit { key, description, kind: LimitKind::Amount { unit }, parts: None }
    }

    /// A limit on usage within period `period`.
    pub const fn metered(key: &'static str, description: &'static str, period: Period) -> Self {
        Limit { key, description, kind: LimitKind::Metered { period }, parts: None }
    }

    pub const fn key(&self) -> &'static str {
        self.key
    }

    pub const fn description(&self) -> &'static str {
        self.description
    }

    pub const fn kind(&self) -> LimitKind {
        self.kind
    }

    /// A limit that also has a separate level per part (e.g. per node type): a plan can set the key
    /// `<key>.<part>` (`organizations.units.store`) in addition to the general level `<key>`. `parts` describes
    /// what a part is (e.g. `"unit type"`), shown to whoever composes plans. A part with no separate level is
    /// not limited separately, only counted toward the general level. The module reserves a part's slot with
    /// [`EntitlementsHandle::reserve_part`] at the same time as the general slot.
    pub const fn per(self, parts: &'static str) -> Self {
        Limit { parts: Some(parts), ..self }
    }

    /// Description of the part, when the limit has a separate level per part.
    pub const fn parts(&self) -> Option<&'static str> {
        self.parts
    }

    /// Key of the separate level of part `part`: `<key>.<part>`.
    pub fn part_key(&self, part: &str) -> String {
        format!("{}.{part}", self.key)
    }
}

/// The real counter in a module's database for a `Count` / `Amount` limit: the reconciliation job uses it to
/// fix a drifted in-use figure (a process died between reserving and writing). A module registers it in
/// `start` with [`StartContext::usage_counter`](crate::module::StartContext::usage_counter).
#[async_trait::async_trait]
pub trait UsageCounter: Send + Sync {
    /// The real amount in use by tenant `tenant`.
    async fn count(&self, tenant: &TenantContext) -> Result<i64, Error>;

    /// The real amount in use per part (a limit that declares [`Limit::per`]): part -> amount. Empty by default
    /// (a limit without parts).
    async fn count_parts(&self, _tenant: &TenantContext) -> Result<BTreeMap<String, i64>, Error> {
        Ok(BTreeMap::new())
    }
}

/// What a tenant may do with its own data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TenantAccess {
    #[default]
    Full,
    /// Subscription temporarily locked or cancelled: read only, no writes.
    ReadOnly,
}

/// Current entitlements of a tenant (plans, add-on plans and overrides merged).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TenantEntitlements {
    /// No limits at all: no subscription and no default plan, or the platform tenant.
    pub unrestricted: bool,
    pub access: TenantAccess,
    /// Enabled modules (not yet including `always_enabled` modules, which are always on).
    pub modules: BTreeSet<String>,
    pub features: BTreeSet<String>,
    /// Limits by key; `None` = unlimited. An absent key = 0 (not allowed), unless `unrestricted`.
    pub limits: BTreeMap<String, Option<i64>>,
}

impl TenantEntitlements {
    /// Unlimited, with everything written out.
    pub fn unrestricted() -> Self {
        TenantEntitlements { unrestricted: true, ..TenantEntitlements::default() }
    }

    /// Whether module `module` is enabled (`always_enabled` not yet considered).
    pub fn has_module(&self, module: &str) -> bool {
        self.unrestricted || self.modules.contains(module)
    }

    pub fn has_feature(&self, feature: &str) -> bool {
        self.unrestricted || self.features.contains(feature)
    }

    /// Separate maximum of a part (`<key>.<part>`, see [`Limit::per`]): `None` = no separate limit. Unlike
    /// [`limit`](Self::limit), an absent key means no separate level is set.
    pub fn part_limit(&self, key: &str) -> Option<i64> {
        if self.unrestricted { None } else { self.limits.get(key).copied().flatten() }
    }

    /// Maximum of limit `limit`: `None` = unlimited.
    pub fn limit(&self, limit: &str) -> Option<i64> {
        if self.unrestricted { None } else { self.limits.get(limit).copied().unwrap_or(Some(0)) }
    }
}

/// The provider of entitlements (module `plans` publishes `dyn Entitlements`). Every function takes the tenant
/// ID: the registry of plans and subscriptions lives at the control plane.
#[async_trait::async_trait]
pub trait Entitlements: Send + Sync {
    /// Current entitlements of tenant `tenant`; may come from a short-lived cache.
    async fn snapshot(&self, tenant: TenantId) -> Result<Arc<TenantEntitlements>, Error>;

    /// Reserves `amount` (> 0) of limit `limit` under `reference` (e.g. `catalog.product:<id>`), with an
    /// expiry on the reservation. Calling again with the same `reference` and the same `amount` does not
    /// reserve more; with a different `amount` it returns `reference_reused`. Exceeding the limit returns
    /// `limit_exceeded`. Do not call it inside a module's write transaction (SQLite has a single writer: it
    /// would lock itself).
    async fn reserve(
        &self,
        tenant: TenantId,
        limit: &Limit,
        amount: i64,
        reference: &str,
    ) -> Result<(), Error>;

    /// The reserved slot becomes permanent (the module's record has been committed). Already confirmed: `Ok`.
    /// No hold for `reference` (it expired, was released by a concurrent call, or `reserve` was never called):
    /// [`hold_missing`] (`limit_hold_missing`) — the caller can match that code and settle again.
    async fn confirm(&self, tenant: TenantId, limit: &Limit, reference: &str) -> Result<(), Error>;

    /// Gives back a reserved or confirmed slot; calling it many times or with an unknown `reference` is always `Ok`.
    async fn release(&self, tenant: TenantId, limit: &Limit, reference: &str) -> Result<(), Error>;

    /// Like [`reserve`](Self::reserve) for the separate level of part `part` (key `<key>.<part>`, see
    /// [`Limit::per`]); a part with no separate level only records usage and is never refused.
    /// Default: unlimited.
    async fn reserve_part(
        &self,
        _tenant: TenantId,
        _limit: &Limit,
        _part: &str,
        _amount: i64,
        _reference: &str,
    ) -> Result<(), Error> {
        Ok(())
    }

    /// Like [`confirm`](Self::confirm) for part `part`. Default: `Ok`.
    async fn confirm_part(
        &self,
        _tenant: TenantId,
        _limit: &Limit,
        _part: &str,
        _reference: &str,
    ) -> Result<(), Error> {
        Ok(())
    }

    /// Like [`release`](Self::release) for part `part`. Default: `Ok`.
    async fn release_part(
        &self,
        _tenant: TenantId,
        _limit: &Limit,
        _part: &str,
        _reference: &str,
    ) -> Result<(), Error> {
        Ok(())
    }

    /// Records usage `quantity` of a `Metered` limit; a duplicate `reference` is ignored. Exceeding the
    /// period's limit returns `quota_exhausted`.
    async fn record_usage(
        &self,
        tenant: TenantId,
        limit: &Limit,
        quantity: i64,
        reference: &str,
    ) -> Result<(), Error>;

    /// Codes of the plans on sale that have `module` / `feature` / a level of limit `limit` greater than
    /// `above` (`None`: unlimited), so the interface can show "which plan to upgrade to in order to unlock"
    /// (step 51 P11). Default: no known plan.
    async fn plans_offering(&self, need: &PlanNeed) -> Result<Vec<String>, Error> {
        let _ = need;
        Ok(Vec::new())
    }

    /// Tenants whose subscription has status `status` (a filter of the tenant list). Default: none.
    async fn tenants_with_status(&self, status: &str) -> Result<Vec<TenantId>, Error> {
        let _ = status;
        Ok(Vec::new())
    }

    /// Tenants that have used 80% or more of a limit (the `near_limit` filter). Default: none.
    async fn tenants_near_limit(&self) -> Result<Vec<TenantId>, Error> {
        Ok(Vec::new())
    }

    /// Subscription (`subscription`) and usage (`usage`) of tenants `tenants`, for the `expand` of the tenant
    /// list, the whole page at once. A tenant that has none is absent from the result. Default: empty.
    async fn describe_tenants(
        &self,
        tenants: &[TenantId],
        subscription: bool,
        usage: bool,
    ) -> Result<BTreeMap<TenantId, TenantPlanView>, Error> {
        let _ = (tenants, subscription, usage);
        Ok(BTreeMap::new())
    }
}

/// What a plan must have (see [`Entitlements::plans_offering`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PlanNeed {
    pub module: Option<String>,
    pub feature: Option<String>,
    /// Limit and its current level: the plan must give more than this level.
    pub limit: Option<(String, i64)>,
}

impl PlanNeed {
    pub fn module(module: &str) -> Self {
        PlanNeed { module: Some(module.to_owned()), ..PlanNeed::default() }
    }

    pub fn feature(feature: &str) -> Self {
        PlanNeed { feature: Some(feature.to_owned()), ..PlanNeed::default() }
    }

    pub fn limit(limit: &str, above: i64) -> Self {
        PlanNeed { limit: Some((limit.to_owned(), above)), ..PlanNeed::default() }
    }
}

/// Subscription and usage of a tenant, in the JSON shape of the providing module (the kernel does not know
/// the exact shape; it only passes it to the `expand` of `GET /api/v1/kernel/tenants`).
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct TenantPlanView {
    pub subscription: Option<serde_json::Value>,
    pub usage: Option<serde_json::Value>,
}

impl TenantPlanView {
    pub fn new(subscription: Option<serde_json::Value>, usage: Option<serde_json::Value>) -> Self {
        TenantPlanView { subscription, usage }
    }
}

/// Handle the kernel gives to EVERY module at `start`. Points to `dyn Entitlements` when a module publishes
/// one; otherwise it means "unlimited". Cheap to clone (an `Arc`).
#[derive(Clone, Default)]
pub struct EntitlementsHandle {
    // `Arc<OnceLock<…>>`: mọi bản clone dùng chung một ô, kernel ghi đúng một lần sau khi mọi
    // module khởi động; đọc từ nhiều thread không cần lock.
    provider: Arc<OnceLock<Arc<dyn Entitlements>>>,
    catalog: EntitlementCatalog,
    meter: Arc<Meter>,
}

/// Monthly API request limit of each tenant (declared by the kernel, counted at the HTTP layer).
pub const API_REQUESTS: Limit = Limit::metered(
    "kernel.api_requests",
    "API requests (REST and MCP) a tenant can make each month.",
    Period::Month,
);

/// Counts API requests per tenant in the process's memory; [`EntitlementsHandle::flush_usage`] writes them
/// merged into the usage ledger (every 60 seconds). If the process dies, at most that much of the count is
/// lost.
#[derive(Default)]
struct Meter {
    // `Mutex` chuẩn: chỉ giữ trong vài phép tính, không qua `.await`.
    counts: std::sync::Mutex<BTreeMap<TenantId, i64>>,
    /// Tenant has used up the period's requests: blocking stops at this point.
    exhausted: std::sync::Mutex<BTreeMap<TenantId, std::time::Instant>>,
}

impl EntitlementsHandle {
    pub(crate) fn new(catalog: EntitlementCatalog) -> Self {
        EntitlementsHandle { provider: Arc::default(), catalog, meter: Arc::default() }
    }

    /// Counts one API request of tenant `tenant` (the HTTP layer calls it). With no provider, or the platform
    /// tenant: no counting.
    pub fn count_request(&self, tenant: TenantId) {
        if self.provider_for(tenant).is_none() {
            return;
        }
        if let Ok(mut counts) = self.meter.counts.lock() {
            *counts.entry(tenant).or_default() += 1;
        }
    }

    /// Tenant `tenant` has used up the period's requests: the number of seconds until the next period, to return 429.
    pub fn request_quota_wait(&self, tenant: TenantId) -> Option<u64> {
        let mut exhausted = self.meter.exhausted.lock().ok()?;
        let until = *exhausted.get(&tenant)?;
        let now = std::time::Instant::now();
        if until <= now {
            exhausted.remove(&tenant);
            return None;
        }
        Some((until - now).as_secs().max(1))
    }

    /// Writes the counted requests into the usage ledger, one row per tenant, `reference` = `node` + time
    /// slot `slot` (e.g. a minute); calling again with the same slot does not write a duplicate. A tenant
    /// that is out of quota: blocked from now until the start of the next period. Other errors: logged, and
    /// that round's count is dropped (the count is for billing, not for holding money).
    pub async fn flush_usage(&self, node: &str, slot: i64) {
        let counts = match self.meter.counts.lock() {
            Ok(mut counts) => std::mem::take(&mut *counts),
            Err(_) => return,
        };
        for (tenant, quantity) in counts {
            let reference = format!("{node}:{slot}");
            match self.record_usage(tenant, &API_REQUESTS, quantity, &reference).await {
                Ok(()) => {}
                Err(error) if error.code() == "quota_exhausted" => {
                    let wait = error.retry_after().unwrap_or(60);
                    let until = std::time::Instant::now() + std::time::Duration::from_secs(wait);
                    if let Ok(mut exhausted) = self.meter.exhausted.lock() {
                        exhausted.insert(tenant, until);
                    }
                }
                Err(error) => {
                    tracing::warn!(tenant_id = %tenant, error = %error, "cannot record API request usage");
                }
            }
        }
    }

    /// Attaches the provider; later calls are ignored.
    pub(crate) fn fill(&self, provider: Option<Arc<dyn Entitlements>>) {
        if let Some(provider) = provider {
            let _ = self.provider.set(provider);
        }
    }

    /// Whether there is an entitlement provider (`plans` is installed).
    pub fn is_enforced(&self) -> bool {
        self.provider.get().is_some()
    }

    /// Registry of every module's features and limits.
    pub fn catalog(&self) -> &EntitlementCatalog {
        &self.catalog
    }

    /// The provider, unless the tenant is exempt (the platform tenant) or there is no provider yet.
    fn provider_for(&self, tenant: TenantId) -> Option<&Arc<dyn Entitlements>> {
        if tenant == DEFAULT_TENANT { None } else { self.provider.get() }
    }

    /// Current entitlements of tenant `tenant`.
    pub async fn snapshot(&self, tenant: TenantId) -> Result<Arc<TenantEntitlements>, Error> {
        match self.provider_for(tenant) {
            Some(provider) => provider.snapshot(tenant).await,
            None => Ok(Arc::new(TenantEntitlements::unrestricted())),
        }
    }

    /// Plans on sale that satisfy `need` (empty when there is no plans module).
    pub async fn plans_offering(
        &self,
        tenant: TenantId,
        need: &PlanNeed,
    ) -> Result<Vec<String>, Error> {
        match self.provider_for(tenant) {
            Some(provider) => provider.plans_offering(need).await,
            None => Ok(Vec::new()),
        }
    }

    /// Whether module `module` is enabled for tenant `tenant` (`always_enabled` modules are always on).
    pub async fn module_enabled(&self, tenant: TenantId, module: &str) -> Result<bool, Error> {
        if self.catalog.is_always_enabled(module) {
            return Ok(true);
        }
        Ok(self.snapshot(tenant).await?.has_module(module))
    }

    /// Like [`module_enabled`](Self::module_enabled), with error `module_not_enabled` when it is not enabled.
    pub async fn require_module(&self, tenant: TenantId, module: &str) -> Result<(), Error> {
        if self.module_enabled(tenant, module).await? {
            Ok(())
        } else {
            Err(module_not_enabled(module))
        }
    }

    pub async fn has_feature(&self, tenant: TenantId, feature: &Feature) -> Result<bool, Error> {
        Ok(self.snapshot(tenant).await?.has_feature(feature.key()))
    }

    /// Error `feature_not_enabled` when the tenant does not have feature `feature`.
    pub async fn require_feature(&self, tenant: TenantId, feature: &Feature) -> Result<(), Error> {
        if self.has_feature(tenant, feature).await? {
            Ok(())
        } else {
            metrics::counter!("entitlement_denials_total", "kind" => "feature").increment(1);
            Err(feature_not_enabled(feature.key()))
        }
    }

    /// See [`Entitlements::tenants_with_status`]; with no provider: none.
    pub async fn tenants_with_status(&self, status: &str) -> Result<Vec<TenantId>, Error> {
        match self.provider.get() {
            Some(provider) => provider.tenants_with_status(status).await,
            None => Ok(Vec::new()),
        }
    }

    /// See [`Entitlements::describe_tenants`]; with no provider: empty.
    pub async fn describe_tenants(
        &self,
        tenants: &[TenantId],
        subscription: bool,
        usage: bool,
    ) -> Result<BTreeMap<TenantId, TenantPlanView>, Error> {
        match self.provider.get() {
            Some(provider) if !tenants.is_empty() && (subscription || usage) => {
                provider.describe_tenants(tenants, subscription, usage).await
            }
            _ => Ok(BTreeMap::new()),
        }
    }

    /// See [`Entitlements::tenants_near_limit`]; with no provider: none.
    pub async fn tenants_near_limit(&self) -> Result<Vec<TenantId>, Error> {
        match self.provider.get() {
            Some(provider) => provider.tenants_near_limit().await,
            None => Ok(Vec::new()),
        }
    }

    /// See [`Entitlements::reserve`]; with no provider it is `Ok`.
    pub async fn reserve(
        &self,
        tenant: TenantId,
        limit: &Limit,
        amount: i64,
        reference: &str,
    ) -> Result<(), Error> {
        match self.provider_for(tenant) {
            Some(provider) => provider.reserve(tenant, limit, amount, reference).await,
            None => Ok(()),
        }
    }

    /// See [`Entitlements::confirm`]; with no provider it is `Ok`.
    pub async fn confirm(
        &self,
        tenant: TenantId,
        limit: &Limit,
        reference: &str,
    ) -> Result<(), Error> {
        match self.provider_for(tenant) {
            Some(provider) => provider.confirm(tenant, limit, reference).await,
            None => Ok(()),
        }
    }

    /// See [`Entitlements::release`]; with no provider it is `Ok`.
    pub async fn release(
        &self,
        tenant: TenantId,
        limit: &Limit,
        reference: &str,
    ) -> Result<(), Error> {
        match self.provider_for(tenant) {
            Some(provider) => provider.release(tenant, limit, reference).await,
            None => Ok(()),
        }
    }

    /// See [`Entitlements::reserve_part`]; with no provider it is `Ok`. A module reserves the general slot
    /// ([`reserve`](Self::reserve)) first, then the part's slot; if the part is refused the general slot is
    /// released.
    pub async fn reserve_part(
        &self,
        tenant: TenantId,
        limit: &Limit,
        part: &str,
        amount: i64,
        reference: &str,
    ) -> Result<(), Error> {
        match self.provider_for(tenant) {
            Some(provider) => provider.reserve_part(tenant, limit, part, amount, reference).await,
            None => Ok(()),
        }
    }

    /// See [`Entitlements::confirm_part`]; with no provider it is `Ok`.
    pub async fn confirm_part(
        &self,
        tenant: TenantId,
        limit: &Limit,
        part: &str,
        reference: &str,
    ) -> Result<(), Error> {
        match self.provider_for(tenant) {
            Some(provider) => provider.confirm_part(tenant, limit, part, reference).await,
            None => Ok(()),
        }
    }

    /// See [`Entitlements::release_part`]; with no provider it is `Ok`.
    pub async fn release_part(
        &self,
        tenant: TenantId,
        limit: &Limit,
        part: &str,
        reference: &str,
    ) -> Result<(), Error> {
        match self.provider_for(tenant) {
            Some(provider) => provider.release_part(tenant, limit, part, reference).await,
            None => Ok(()),
        }
    }

    /// See [`Entitlements::record_usage`]; with no provider it is `Ok`.
    pub async fn record_usage(
        &self,
        tenant: TenantId,
        limit: &Limit,
        quantity: i64,
        reference: &str,
    ) -> Result<(), Error> {
        match self.provider_for(tenant) {
            Some(provider) => provider.record_usage(tenant, limit, quantity, reference).await,
            None => Ok(()),
        }
    }
}

/// A limit in the registry, with the module that declared it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogLimit {
    pub module: &'static str,
    pub limit: Limit,
}

/// A feature in the registry, with the module that declared it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogFeature {
    pub module: &'static str,
    pub feature: Feature,
}

#[derive(Default)]
struct CatalogData {
    modules: Vec<&'static str>,
    always_enabled: BTreeSet<&'static str>,
    features: BTreeMap<&'static str, CatalogFeature>,
    limits: BTreeMap<&'static str, CatalogLimit>,
}

/// Registry of features, limits and enableable modules of every module: built at registration (static
/// declarations), with the `requires` part and the counters filled in after every module has started.
/// Cheap to clone.
#[derive(Clone, Default)]
pub struct EntitlementCatalog {
    data: Arc<CatalogData>,
    // Phần điền sau `start`.
    late: Arc<OnceLock<Late>>,
}

#[derive(Default)]
struct Late {
    /// `(module, module it needs)`: recorded when a module calls `StartContext::require`.
    requires: BTreeSet<(&'static str, &'static str)>,
    counters: BTreeMap<&'static str, Arc<dyn UsageCounter>>,
}

impl EntitlementCatalog {
    /// Builds the registry from the modules' declarations; also returns the declaration errors (a key that
    /// does not carry the module's prefix, a duplicate key, a malformed key).
    pub(crate) fn build(
        declared: Vec<(&'static str, bool, Vec<Feature>, Vec<Limit>)>,
    ) -> (Self, Vec<String>) {
        let mut problems = Vec::new();
        let mut data = CatalogData::default();
        for (module, always, features, limits) in declared {
            data.modules.push(module);
            if always {
                data.always_enabled.insert(module);
            }
            for feature in features {
                problems.extend(key_problem(module, feature.key(), "feature"));
                if data.features.insert(feature.key(), CatalogFeature { module, feature }).is_some()
                {
                    problems.push(format!("feature `{}` is declared twice", feature.key()));
                }
            }
            for limit in limits {
                problems.extend(key_problem(module, limit.key(), "limit"));
                if data.limits.insert(limit.key(), CatalogLimit { module, limit }).is_some() {
                    problems.push(format!("limit `{}` is declared twice", limit.key()));
                }
            }
        }
        // Hai module nội bộ của kernel luôn bật.
        data.always_enabled.extend([crate::builtin::NAME, crate::antispam::module::NAME]);
        (EntitlementCatalog { data: Arc::new(data), late: Arc::default() }, problems)
    }

    pub(crate) fn fill(
        &self,
        requires: BTreeSet<(&'static str, &'static str)>,
        counters: BTreeMap<&'static str, Arc<dyn UsageCounter>>,
    ) {
        let _ = self.late.set(Late { requires, counters });
    }

    /// Every registered module, in order.
    pub fn modules(&self) -> &[&'static str] {
        &self.data.modules
    }

    /// Modules that are never blocked by a plan.
    pub fn is_always_enabled(&self, module: &str) -> bool {
        self.data.always_enabled.contains(module)
    }

    pub fn features(&self) -> impl Iterator<Item = &CatalogFeature> {
        self.data.features.values()
    }

    pub fn feature(&self, key: &str) -> Option<&CatalogFeature> {
        self.data.features.get(key)
    }

    pub fn limits(&self) -> impl Iterator<Item = &CatalogLimit> {
        self.data.limits.values()
    }

    /// Limit with key `key`, including the key of a part's separate level (`<key>.<part>`) of a limit that
    /// declares [`Limit::per`]; the part must be an identifier-shaped name (lowercase letters, digits, `_`).
    pub fn limit(&self, key: &str) -> Option<&CatalogLimit> {
        if let Some(found) = self.data.limits.get(key) {
            return Some(found);
        }
        let (base, part) = key.rsplit_once('.')?;
        let found = self.data.limits.get(base)?;
        let valid = found.limit.parts().is_some() && crate::module::is_identifier(part);
        valid.then_some(found)
    }

    /// Modules that `module` needs (it did `require` their traits at startup). Empty before every module has
    /// started.
    pub fn requires(&self, module: &str) -> Vec<&'static str> {
        self.late
            .get()
            .map(|late| {
                late.requires
                    .iter()
                    .filter(|(who, _)| *who == module)
                    .map(|(_, needed)| *needed)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The real counter of limit `key`, if the module registered one.
    pub fn counter(&self, key: &str) -> Option<Arc<dyn UsageCounter>> {
        self.late.get().and_then(|late| late.counters.get(key).cloned())
    }
}

/// Declaration error when `key` does not have the form `<module>.<name>`.
fn key_problem(module: &str, key: &str, what: &str) -> Option<String> {
    let owned = key.strip_prefix(module).and_then(|rest| rest.strip_prefix('.'));
    let well_formed = owned
        .is_some_and(|rest| !rest.is_empty() && rest.split('.').all(crate::module::is_identifier));
    (!well_formed).then(|| {
        format!(
            "module `{module}` declares {what} `{key}`; it must be named `{module}.<name>` \
             (lowercase letters, digits, `_`, parts separated by `.`)"
        )
    })
}

/// `module_not_enabled` (403): the tenant's plan does not include this module.
pub fn module_not_enabled(module: &str) -> Error {
    Error::forbidden(
        "module_not_enabled",
        "Your current plan does not include the module `{module}`. Upgrade the plan to use it.",
    )
    .with_param("module", module.to_owned())
}

/// `feature_not_enabled` (403): the tenant's plan does not include this feature.
pub fn feature_not_enabled(feature: &str) -> Error {
    Error::forbidden(
        "feature_not_enabled",
        "Your current plan does not include the feature `{feature}`.",
    )
    .with_param("feature", feature.to_owned())
}

/// `limit_exceeded` (403): the limit has been used up.
pub fn limit_exceeded(limit: &str, used: i64, max: i64) -> Error {
    metrics::counter!("entitlement_denials_total", "kind" => "limit").increment(1);
    Error::forbidden(
        "limit_exceeded",
        "The limit `{limit}` is used up ({used}/{max}). Upgrade the plan or free some up.",
    )
    .with_param("limit", limit.to_owned())
    .with_param("used", used)
    .with_param("max", max)
}

/// `limit_hold_missing` (409): [`Entitlements::confirm`] found no hold for `reference` — it expired, a
/// concurrent call released it, or `reserve` was not called.
pub fn hold_missing(limit: &str, reference: &str) -> Error {
    Error::conflict(
        "limit_hold_missing",
        "There is no hold on the limit `{limit}` for `{reference}` (it expired or was released); reserve again.",
    )
    .with_param("limit", limit.to_owned())
    .with_param("reference", reference.to_owned())
}

/// `quota_exhausted` (429): this period's quota has been used up; a new period starts after `retry_after` seconds.
pub fn quota_exhausted(limit: &str, retry_after: u64) -> Error {
    metrics::counter!("entitlement_denials_total", "kind" => "quota").increment(1);
    Error::too_many_requests(
        "quota_exhausted",
        "The `{limit}` of this period is used up; a new period starts in {retry_after} seconds.",
    )
    .with_param("limit", limit.to_owned())
    .with_param("retry_after", retry_after)
    .with_retry_after(retry_after)
}

/// `reference_reused` (409): the same reference already reserved a slot for a different amount.
pub fn reference_reused(reference: &str, limit: &str) -> Error {
    Error::conflict(
        "reference_reused",
        "The reference `{reference}` was already used for another amount of `{limit}`.",
    )
    .with_param("reference", reference.to_owned())
    .with_param("limit", limit.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHOP_FEATURE: Feature = Feature::new("shop.coupons", "Coupons.");
    const SHOP_LIMIT: Limit = Limit::count("shop.products", "Products.");

    #[test]
    fn khoa_phai_mang_ten_module_va_khong_trung() {
        let (catalog, problems) = EntitlementCatalog::build(vec![
            ("shop", false, vec![SHOP_FEATURE], vec![SHOP_LIMIT]),
            ("blog", true, vec![Feature::new("shop.posts", "x")], vec![Limit::count("blog", "x")]),
            ("news", false, vec![], vec![SHOP_LIMIT]),
        ]);
        assert_eq!(problems.len(), 4, "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("`shop.posts`")));
        assert!(problems.iter().any(|p| p.contains("`blog`")));
        assert!(problems.iter().any(|p| p.contains("declared twice")));
        assert!(catalog.is_always_enabled("blog"));
        assert!(catalog.is_always_enabled("kernel"));
        assert!(!catalog.is_always_enabled("shop"));
        assert_eq!(catalog.feature("shop.coupons").unwrap().module, "shop");
    }

    #[test]
    fn quyen_dung_khong_gioi_han_va_co_gioi_han() {
        let open = TenantEntitlements::unrestricted();
        assert!(open.has_module("shop") && open.has_feature("shop.coupons"));
        assert_eq!(open.limit("shop.products"), None);
        let mut limited = TenantEntitlements::default();
        limited.modules.insert("shop".to_owned());
        limited.limits.insert("shop.products".to_owned(), Some(10));
        limited.limits.insert("shop.orders".to_owned(), None);
        assert!(limited.has_module("shop") && !limited.has_module("blog"));
        assert!(!limited.has_feature("shop.coupons"));
        assert_eq!(limited.limit("shop.products"), Some(10));
        assert_eq!(limited.limit("shop.orders"), None);
        assert_eq!(limited.limit("shop.unknown"), Some(0));
    }

    #[tokio::test]
    async fn khong_co_ben_cung_cap_thi_khong_gioi_han() {
        let (catalog, _) = EntitlementCatalog::build(vec![("shop", false, vec![], vec![])]);
        let handle = EntitlementsHandle::new(catalog);
        let tenant = TenantId::from_uuid(uuid::Uuid::now_v7());
        assert!(!handle.is_enforced());
        assert!(handle.module_enabled(tenant, "shop").await.unwrap());
        handle.require_feature(tenant, &SHOP_FEATURE).await.unwrap();
        handle.reserve(tenant, &SHOP_LIMIT, 5, "shop.product:1").await.unwrap();
    }
}
