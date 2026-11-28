//! One settings system for a tenant (D-20261128-settings-one-system): the per-tenant settings ([`TenantSetting`],
//! stored by the kernel) and the settings inherited down the organization tree ([`OrgSetting`], whose values the
//! `organizations` module stores) are two **scopes** of one system, served by one kernel service:
//!
//! - **One read path with a cache.** [`SettingSystem::effective`] gives the effective value of a per-node setting:
//!   the value set on the nearest node of the ancestor chain, or the default. The values come from the store the
//!   owning module plugs in ([`OrgSettingStore`]); the stored values and each node's ancestor chain are cached per
//!   tenant for `AUTOXIDE_TENANCY_CACHE_SECONDS`, like the per-tenant settings, so a read that hits the cache runs
//!   no query. A write on this server clears the tenant's cache at once ([`SettingSystem::changed`]).
//! - **Cross-checks when saving.** A module registers a [`SettingRule`] over keys of either scope; saving one of
//!   those keys runs the rule on the values as they would be after the change ([`SettingsView`]) and answers 422
//!   when it fails, so a broken pair (a default language missing from the language list) is refused when saved
//!   instead of being skipped when used. Rules run inside the write transaction after a per-tenant lock, so two
//!   saves at the same time cannot both pass a rule and leave a broken pair.
//!
//! Which scope a new setting goes in: technical configuration and partner connections of the whole tenant are
//! per-tenant settings; business policy that may differ per store or branch is a per-node setting. A key lives in
//! one scope only.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError, RwLock};
use std::time::Instant;

use sea_orm::{ColumnTrait as _, EntityTrait as _, QueryFilter as _};
use serde_json::Value;

use super::{SettingsService, entity};
use crate::error::{Error, FieldError};
use crate::tenancy::{
    OrgId, OrgSettingCatalog, OrgTree, TenantContext, TenantId, nearest, root_of,
};

/// One value of a per-node setting as its store keeps it.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct StoredOrgSetting {
    /// The node that sets it.
    pub org: OrgId,
    /// The setting's key (`<module>.<name>`).
    pub key: String,
    /// The value, already accepted by the setting's checker.
    pub value: Value,
}

impl StoredOrgSetting {
    /// Value `value` of setting `key` set on node `org`.
    pub fn new(org: OrgId, key: impl Into<String>, value: Value) -> Self {
        StoredOrgSetting { org, key: key.into(), value }
    }
}

/// Where the values of the per-node settings are stored: plugged in by the module that owns them
/// (`organizations`) with [`SettingSystem::use_org_store`].
//
// `#[async_trait]`: hàm `async` gọi qua `dyn Trait`.
#[async_trait::async_trait]
pub trait OrgSettingStore: Send + Sync {
    /// Every value set on any node of tenant `tenant` (one query).
    async fn stored(&self, tenant: &TenantContext) -> Result<Vec<StoredOrgSetting>, Error>;
}

/// The effective value of a per-node setting at a node.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct EffectiveOrgSetting {
    /// The setting's key.
    pub key: String,
    /// The value in force: set on the node or an ancestor, otherwise the default; `None` when neither.
    pub value: Option<Value>,
    /// The node whose value is in force; `None` when it is the default.
    pub source: Option<OrgId>,
    /// `false` only when the node itself sets the value.
    pub inherited: bool,
}

type RuleCheck = dyn Fn(&SettingsView<'_>) -> Result<(), FieldError> + Send + Sync;

/// A cross-check between settings, run when one of its keys is saved: `check` sees the values as they would be
/// after the change and returns the problem to report (field `value`). Keys may be per-tenant setting names or
/// per-node setting keys, of any module; a secret setting cannot be part of a rule.
pub struct SettingRule {
    keys: Vec<&'static str>,
    check: Box<RuleCheck>,
}

impl std::fmt::Debug for SettingRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingRule").field("keys", &self.keys).finish_non_exhaustive()
    }
}

impl SettingRule {
    /// A rule over `keys`: `check` returns `Err` with what to fix when the values do not fit together.
    pub fn new(
        keys: &[&'static str],
        check: impl Fn(&SettingsView<'_>) -> Result<(), FieldError> + Send + Sync + 'static,
    ) -> Self {
        SettingRule { keys: keys.to_vec(), check: Box::new(check) }
    }

    /// The keys the rule binds.
    pub fn keys(&self) -> &[&'static str] {
        &self.keys
    }
}

/// The values a [`SettingRule`] checks: as they would be after the change being saved.
#[derive(Debug)]
pub struct SettingsView<'a> {
    tenant: &'a HashMap<String, String>,
    org: &'a HashMap<String, Value>,
}

impl SettingsView<'_> {
    /// The value of per-tenant setting `name` as set by the tenant; `None` when unset (the module then uses the
    /// server-wide configuration).
    pub fn text(&self, name: &str) -> Option<&str> {
        self.tenant.get(name).map(String::as_str)
    }

    /// The effective value of per-node setting `key` at the node being changed (the root node when a per-tenant
    /// setting is being changed), default included.
    pub fn value(&self, key: &str) -> Option<&Value> {
        self.org.get(key)
    }
}

/// The tree part of the settings service (kept inside [`SettingsService`]).
#[derive(Default)]
pub(super) struct Tree {
    catalog: OnceLock<OrgSettingCatalog>,
    store: OnceLock<Arc<dyn OrgSettingStore>>,
    rules: RwLock<Vec<Arc<SettingRule>>>,
    // `Mutex` chuẩn: chỉ giữ trong vài phép tính, không qua `.await`.
    cache: Mutex<HashMap<TenantId, (Instant, Arc<TreeEntry>)>>,
}

/// What the cache keeps of one tenant: the stored values and the ancestor chains asked so far.
struct TreeEntry {
    rows: Vec<StoredOrgSetting>,
    lineage: Mutex<HashMap<OrgId, Arc<Vec<OrgId>>>>,
}

impl Tree {
    pub(super) fn forget(&self, tenant: TenantId) {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner).remove(&tenant);
    }

    fn rules_for(&self, key: &str) -> Vec<Arc<SettingRule>> {
        let rules = self.rules.read().unwrap_or_else(PoisonError::into_inner);
        rules.iter().filter(|rule| rule.keys.contains(&key)).cloned().collect()
    }

    fn catalog(&self) -> OrgSettingCatalog {
        self.catalog.get().cloned().unwrap_or_default()
    }
}

/// The settings system of the tenants, published by the kernel: `context.require::<SettingSystem>()`. Cheap to
/// clone. See the module documentation.
#[derive(Clone)]
pub struct SettingSystem(Arc<SettingsService>);

impl std::fmt::Debug for SettingSystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingSystem").finish_non_exhaustive()
    }
}

/// Name of the per-tenant lock taken before cross-checked settings are saved.
fn lock_name(tenant: TenantId) -> String {
    format!("kernel.settings:{tenant}")
}

impl SettingSystem {
    pub(crate) fn new(service: Arc<SettingsService>, catalog: OrgSettingCatalog) -> Self {
        let _ = service.tree.catalog.set(catalog);
        SettingSystem(service)
    }

    /// Plugs in the store of the per-node values (once; the module that owns them calls it in `start`).
    pub fn use_org_store(&self, store: Arc<dyn OrgSettingStore>) -> Result<(), Error> {
        self.0
            .tree
            .store
            .set(store)
            .map_err(|_| Error::internal("a store of the per-node settings is already plugged in"))
    }

    /// Registers a cross-check (in `start`). Every key must be a declared setting of either scope and not secret.
    pub fn rule(&self, rule: SettingRule) -> Result<(), Error> {
        let catalog = self.0.tree.catalog();
        for key in &rule.keys {
            let tenant = self.0.catalog.find(key).ok();
            if tenant.is_some_and(|setting| setting.secret) {
                return Err(Error::internal(format!(
                    "setting rule over `{key}`: a secret setting cannot be cross-checked"
                )));
            }
            if tenant.is_none() && catalog.get(key).is_none() {
                return Err(Error::internal(format!(
                    "setting rule over `{key}`, which no module declares"
                )));
            }
        }
        self.0.tree.rules.write().unwrap_or_else(PoisonError::into_inner).push(Arc::new(rule));
        Ok(())
    }

    /// Whether saving `key` runs a cross-check (then the saver calls [`lock`](Self::lock) and
    /// [`check_org_change`](Self::check_org_change)).
    pub fn has_rules(&self, key: &str) -> bool {
        !self.0.tree.rules_for(key).is_empty()
    }

    /// Takes the per-tenant settings lock until `transaction` ends: a saver of a cross-checked per-node setting
    /// calls it first in its write transaction, before reading the values it passes to
    /// [`check_org_change`](Self::check_org_change).
    pub async fn lock(
        &self,
        transaction: &impl crate::db::Executor,
        tenant: &TenantContext,
    ) -> Result<(), Error> {
        let via = crate::db::Via::of(transaction);
        Ok(crate::rows::lock_key(&via, &lock_name(tenant.id())).await?)
    }

    /// Runs the cross-checks of per-node setting `key` before node `node` sets it to `value` (`None`: removes it).
    /// `stored`: every per-node value of the tenant before the change, read in `transaction` after
    /// [`lock`](Self::lock). `Err` 422 naming what to fix when a rule fails.
    pub async fn check_org_change(
        &self,
        transaction: &impl crate::db::Executor,
        tenant: &TenantContext,
        node: OrgId,
        key: &str,
        value: Option<&Value>,
        mut stored: Vec<StoredOrgSetting>,
    ) -> Result<(), Error> {
        let rules = self.0.tree.rules_for(key);
        if rules.is_empty() {
            return Ok(());
        }
        stored.retain(|row| !(row.org == node && row.key == key));
        if let Some(value) = value {
            stored.push(StoredOrgSetting::new(node, key, value.clone()));
        }
        let via = crate::db::Via::of(transaction);
        let tenant_values = plain_values(&via, tenant).await?;
        // Chuỗi tổ tiên đọc trong transaction; luôn có chính nút (nút gốc không có dòng tổ tiên nào khác).
        let mut lineage = if node == root_of(tenant.id()) {
            vec![node]
        } else {
            OrgTree::ancestors(transaction, tenant.id(), node).await?
        };
        if lineage.is_empty() {
            lineage.push(node);
        }
        let org = view_values(&self.0.tree.catalog(), &stored, &lineage);
        run(&rules, &tenant_values, &org)
    }

    /// The effective value of per-node setting `key` at node `node`; `None` when no module declares `key`.
    pub async fn effective(
        &self,
        tenant: &TenantContext,
        node: OrgId,
        key: &str,
    ) -> Result<Option<EffectiveOrgSetting>, Error> {
        if self.0.tree.catalog().get(key).is_none() {
            return Ok(None);
        }
        Ok(self.resolve(tenant, node, Some(key)).await?.into_iter().next())
    }

    /// The effective value of every per-node setting at node `node`, in key order.
    pub async fn effective_all(
        &self,
        tenant: &TenantContext,
        node: OrgId,
    ) -> Result<Vec<EffectiveOrgSetting>, Error> {
        self.resolve(tenant, node, None).await
    }

    /// The per-node values of tenant `tenant` changed on this server (or a node moved): drops its cache. The
    /// owning module calls it after its write commits.
    pub fn changed(&self, tenant: TenantId) {
        self.0.forget(tenant);
    }

    async fn resolve(
        &self,
        tenant: &TenantContext,
        node: OrgId,
        only: Option<&str>,
    ) -> Result<Vec<EffectiveOrgSetting>, Error> {
        let entry = self.entry(tenant).await?;
        let lineage = lineage(&entry, tenant, node).await?;
        Ok(resolve(&self.0.tree.catalog(), &entry.rows, &lineage, node, only))
    }

    /// The tenant's cached values, loaded through the store on a miss. The `mutex` lock is held only within each
    /// statement, never across an `await`.
    async fn entry(&self, tenant: &TenantContext) -> Result<Arc<TreeEntry>, Error> {
        let tree = &self.0.tree;
        let cached =
            tree.cache.lock().unwrap_or_else(PoisonError::into_inner).get(&tenant.id()).cloned();
        if let Some((at, entry)) = cached
            && at.elapsed() < self.0.ttl
        {
            metrics::counter!("kernel_setting_cache_total", "result" => "hit").increment(1);
            return Ok(entry);
        }
        metrics::counter!("kernel_setting_cache_total", "result" => "miss").increment(1);
        let generation = self.0.generation(tenant.id());
        let rows = match tree.store.get() {
            Some(store) => store.stored(tenant).await?,
            None => Vec::new(),
        };
        let entry = Arc::new(TreeEntry { rows, lineage: Mutex::default() });
        let mut cache = tree.cache.lock().unwrap_or_else(PoisonError::into_inner);
        if cache.len() >= super::CACHE_PRUNE_AT {
            let ttl = self.0.ttl;
            cache.retain(|_, (at, _)| at.elapsed() < ttl);
        }
        // Có lần ghi (`changed`) trong lúc đọc kho: không giữ bản vừa đọc, để không đè giá trị mới bằng bản cũ.
        if self.0.generation(tenant.id()) == generation {
            cache.insert(tenant.id(), (Instant::now(), entry.clone()));
        }
        Ok(entry)
    }
}

// Chuỗi tổ tiên của `node` (gần nhất trước), từ cache của khách hay một câu đọc. Nút gốc không có tổ tiên: không cần
// đọc. Chuỗi rỗng (nút không có / chưa có) không giữ lại, để nút vừa tạo không bị nhớ nhầm là không có tổ tiên.
async fn lineage(
    entry: &TreeEntry,
    tenant: &TenantContext,
    node: OrgId,
) -> Result<Arc<Vec<OrgId>>, Error> {
    if node == root_of(tenant.id()) {
        return Ok(Arc::new(vec![node]));
    }
    if let Some(found) = entry.lineage.lock().unwrap_or_else(PoisonError::into_inner).get(&node) {
        return Ok(found.clone());
    }
    let chain = Arc::new(OrgTree::ancestors(tenant.database().read(), tenant.id(), node).await?);
    if !chain.is_empty() {
        entry.lineage.lock().unwrap_or_else(PoisonError::into_inner).insert(node, chain.clone());
    }
    Ok(chain)
}

// Giá trị hiệu lực của từng cài đặt đã khai (hay chỉ `only`) tại `node`, theo chuỗi `lineage`.
fn resolve(
    catalog: &OrgSettingCatalog,
    rows: &[StoredOrgSetting],
    lineage: &[OrgId],
    node: OrgId,
    only: Option<&str>,
) -> Vec<EffectiveOrgSetting> {
    catalog
        .iter()
        .filter(|declared| only.is_none_or(|key| key == declared.key()))
        .map(|declared| {
            let values = rows
                .iter()
                .filter(|row| row.key == declared.key())
                .map(|row| (row.org, row.value.clone()));
            match nearest(lineage, values) {
                Some((source, value)) => EffectiveOrgSetting {
                    key: declared.key().to_owned(),
                    value: Some(value),
                    source: Some(source),
                    inherited: source != node,
                },
                None => EffectiveOrgSetting {
                    key: declared.key().to_owned(),
                    value: declared.default_value(),
                    source: None,
                    inherited: true,
                },
            }
        })
        .collect()
}

fn view_values(
    catalog: &OrgSettingCatalog,
    rows: &[StoredOrgSetting],
    lineage: &[OrgId],
) -> HashMap<String, Value> {
    let node = lineage.first().copied().unwrap_or_else(|| OrgId::from_uuid(uuid::Uuid::nil()));
    resolve(catalog, rows, lineage, node, None)
        .into_iter()
        .filter_map(|effective| Some((effective.key, effective.value?)))
        .collect()
}

// Giá trị không phải secret mà khách đã đặt, đọc qua `connection` (trong transaction của người lưu).
async fn plain_values<C: sea_orm::ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
) -> Result<HashMap<String, String>, Error> {
    let rows = entity::Entity::find()
        .filter(entity::Column::TenantId.eq(tenant.id().as_uuid()))
        .filter(entity::Column::Secret.eq(false))
        .all(connection)
        .await?;
    Ok(rows.into_iter().map(|row| (row.name, row.value)).collect())
}

fn run(
    rules: &[Arc<SettingRule>],
    tenant: &HashMap<String, String>,
    org: &HashMap<String, Value>,
) -> Result<(), Error> {
    let view = SettingsView { tenant, org };
    let problems: Vec<FieldError> = rules
        .iter()
        .filter_map(|rule| (rule.check)(&view).err())
        .map(|mut problem| {
            problem.field = "value".to_owned();
            problem
        })
        .collect();
    if problems.is_empty() { Ok(()) } else { Err(Error::validation(problems)) }
}

impl SettingsService {
    /// Runs the cross-checks of per-tenant setting `name` inside `transaction` before it is set to `value`
    /// (`None`: removed). Takes the per-tenant lock first, so two saves cannot both pass a rule.
    pub(super) async fn check_tenant_change(
        &self,
        transaction: &sea_orm::DatabaseTransaction,
        tenant: &TenantContext,
        setting: &super::TenantSetting,
        value: Option<&str>,
    ) -> Result<(), Error> {
        let rules = self.tree.rules_for(setting.name);
        if rules.is_empty() {
            return Ok(());
        }
        crate::rows::lock_key(transaction, &lock_name(tenant.id())).await?;
        let mut values = plain_values(transaction, tenant).await?;
        // Giá trị dưới tên cũ được đọc như tên mới; lần lưu này thay cả hai.
        if let Some(former) = setting.former_name
            && let Some(old) = values.remove(former)
        {
            values.entry(setting.name.to_owned()).or_insert(old);
        }
        match value {
            Some(value) => values.insert(setting.name.to_owned(), value.to_owned()),
            None => values.remove(setting.name),
        };
        // Phần theo cây: giá trị hiệu lực tại nút gốc, đã commit (từ kho) — chỉ đọc khi một luật có khoá theo cây.
        let catalog = self.tree.catalog();
        let needs_tree =
            rules.iter().any(|rule| rule.keys.iter().any(|key| catalog.get(key).is_some()));
        let org = match self.tree.store.get().filter(|_| needs_tree) {
            Some(store) => {
                let rows = store.stored(tenant).await?;
                view_values(&self.tree.catalog(), &rows, &[root_of(tenant.id())])
            }
            None => view_values(&self.tree.catalog(), &[], &[root_of(tenant.id())]),
        };
        run(&rules, &values, &org)
    }
}
