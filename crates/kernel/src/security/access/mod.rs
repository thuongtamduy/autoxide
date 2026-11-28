//! Access control: permissions declared by modules, and "who is calling" ([`Principal`]).
//!
//! - Each module declares its permissions through `Module::permissions`. Names are written
//!   `<module>.<action>` (e.g. `notes.write`, `auth.roles.manage`); the core validates names at
//!   startup and collects them into one catalog ([`PermissionCatalog`]).
//! - One module (usually `auth`) publishes the trait [`Authenticator`]: it reads the
//!   `Authorization` header and returns the [`Principal`]. The kernel attaches it to every request.
//! - A handler that needs sign-in takes `principal: Principal` as a parameter, then calls
//!   `principal.require(&NOTES_WRITE)?`. A missing permission returns 403 `permission_denied`.
//! - A public endpoint (anyone can call it, signed-in users see more) takes `principal: Option<Principal>`:
//!   without `Authorization` it is `None`; if one is sent but is wrong it is still 401 like a normal endpoint
//!   — it is not silently treated as a guest. The OpenAPI documentation of that handler declares
//!   `security((), ("bearer" = []))`.
//!
//! This way business modules check permissions without depending on the `auth` module, and the `auth`
//! module can be replaced by another authentication method without any module needing changes.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::sync::Arc;

use axum::extract::{FromRequestParts, OptionalFromRequestParts};
use axum::http::header;
use axum::http::request::Parts;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

mod any_of;
mod authenticator;
mod bound;
mod decision;
mod oauth;
#[cfg(test)]
mod properties;
mod step_up;

pub use authenticator::Authenticator;
pub use decision::Decision;
pub use oauth::{AuthorizationServer, Credential, OAuthAccess, canonical_resource, scope_allows};
pub use step_up::StepUp;

use crate::error::Error;
use crate::module::is_identifier;
use crate::tenancy::TenantContext;
use crate::tenancy::{DEFAULT_TENANT, OrgId, OrgTree, TenantId, root_of};

/// A permission declared by a module.
///
/// ```
/// use autoxide_kernel::access::Permission;
///
/// pub const NOTES_WRITE: Permission =
///     Permission::new("notes.write", "Create, change and delete notes.");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Permission {
    name: &'static str,
    description: &'static str,
    risk: Risk,
    /// Applies to data that has `org_id` (step 34): a role can grant it with a depth narrower than `tenant`.
    scoped: bool,
}

// Độ sâu của một quyền trong cây tổ chức (bước 34, như "access level" của Dynamics 365). Thứ tự
// khai báo là thứ tự rộng dần: `own < unit < subtree < tenant`. Với membership tại nút N: `own` —
// bản ghi mình sở hữu; `unit` — bản ghi của đúng nút N; `subtree` — bản ghi của N và mọi nút dưới
// N; `tenant` — mọi bản ghi của khách.
/// How far a permission reaches in the organization tree, from the unit of the membership:
/// `own` (records the person owns), `unit` (that unit), `subtree` (that unit and every unit below
/// it), `tenant` (the whole tenant).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// Only records the person owns.
    Own,
    /// Records of the unit of the membership.
    Unit,
    /// Records of the unit of the membership and of every unit below it.
    Subtree,
    /// Every record of the tenant.
    Tenant,
}

impl Scope {
    /// Stable name in the API and database: `own`, `unit`, `subtree`, `tenant`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Scope::Own => "own",
            Scope::Unit => "unit",
            Scope::Subtree => "subtree",
            Scope::Tenant => "tenant",
        }
    }

    /// Reads from the stable name; an unknown name → `None`.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "own" => Some(Scope::Own),
            "unit" => Some(Scope::Unit),
            "subtree" => Some(Scope::Subtree),
            "tenant" => Some(Scope::Tenant),
            _ => None,
        }
    }
}

/// A concrete permission: at node `org`, depth `scope`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Grant {
    pub org: OrgId,
    pub scope: Scope,
}

impl Grant {
    pub const fn new(org: OrgId, scope: Scope) -> Self {
        Grant { org, scope }
    }
}

/// Target of a permission check on a record: the node that owns the record and the owner (if the record
/// has the notion of an owner — depth `own` needs it; without one `own` is treated like `unit`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrgTarget {
    pub org: OrgId,
    pub owner: Option<Uuid>,
}

impl OrgTarget {
    pub const fn new(org: OrgId, owner: Option<Uuid>) -> Self {
        OrgTarget { org, owner }
    }
}

// Loại tài khoản (bước 34): nhân sự (`staff`) có permission; người mua (`customer`) là tài khoản
// của một khách, không có permission nào.
/// The kind of account: `staff` (has the permissions of their roles) or `customer` (an account of
/// one tenant's shop, without permissions).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Realm {
    /// Staff of a tenant: has the permissions of their roles.
    #[default]
    Staff,
    /// A customer account of one tenant: signs in, has no permission.
    Customer,
}

/// Kind of actor (step 51 P12, D-224): written to the audit (`actor_kind`), used to require approval of risky
/// commands when an agent calls. A signed-in user is `Human` (the value `user` in the audit, kept as before).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    /// A person (signed in, or through an API key of kind `human`).
    #[default]
    Human,
    /// An AI agent, through an API key of kind `agent`.
    Agent,
    /// Another program, through an API key of kind `service`.
    Service,
    /// The program itself (jobs, CLI).
    System,
    /// Platform staff inside the tenant through a support grant.
    Support,
    /// An installed app.
    App,
    /// Provisioning through SCIM.
    Scim,
}

impl ActorKind {
    /// Value of the `actor_kind` column of events / audit. `Human` is `user` (the value that existed before step 51).
    pub const fn as_str(self) -> &'static str {
        match self {
            ActorKind::Human => "user",
            ActorKind::Agent => "agent",
            ActorKind::Service => "service",
            ActorKind::System => "system",
            ActorKind::Support => "support",
            ActorKind::App => "app",
            ActorKind::Scim => "scim",
        }
    }

    /// Inverse of [`as_str`](Self::as_str); `human` is also `Human`.
    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "user" | "human" => ActorKind::Human,
            "agent" => ActorKind::Agent,
            "service" => ActorKind::Service,
            "system" => ActorKind::System,
            "support" => ActorKind::Support,
            "app" => ActorKind::App,
            "scim" => ActorKind::Scim,
            _ => return None,
        })
    }
}

/// The route by which the caller enters the current tenant (step 34): direct membership, delegation from the
/// parent tenant, or a support access grant of platform staff. Recorded in events (`Actor::access`) so the
/// audit can see it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
#[non_exhaustive]
pub enum AccessPath {
    #[default]
    Direct,
    /// Through the delegation with this ID.
    Delegation(Uuid),
    /// Through the support access grant with this ID.
    Support(Uuid),
    /// Action of an automation rule (step 58), in the run with this ID: the rule runs with the permissions of
    /// its creator, and the events it produces carry the run so the `automations` module can prevent loops
    /// between rules.
    Automation(Uuid),
    /// An OAuth client acting for the user through the grant with this ID (step 71 G4). Only in events
    /// (`Actor::access`): the caller itself entered the tenant directly.
    OAuthGrant(Uuid),
}

impl AccessPath {
    /// Stable name: `direct`, `delegation`, `support`, `automation`, `oauth_grant`.
    pub const fn as_str(self) -> &'static str {
        match self {
            AccessPath::Direct => "direct",
            AccessPath::Delegation(_) => "delegation",
            AccessPath::Support(_) => "support",
            AccessPath::Automation(_) => "automation",
            AccessPath::OAuthGrant(_) => "oauth_grant",
        }
    }

    /// ID of the delegation / support grant; `None` when direct.
    pub const fn id(self) -> Option<Uuid> {
        match self {
            AccessPath::Direct => None,
            AccessPath::Delegation(id)
            | AccessPath::Support(id)
            | AccessPath::Automation(id)
            | AccessPath::OAuthGrant(id) => Some(id),
        }
    }
}

/// Risk level of a permission, machine-readable (D-205, spec 54 P1): the UI and AI agents know in advance
/// which actions are read-only and which need care; a read-only MCP tool may only require a permission of
/// level [`Risk::Read`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Risk {
    /// View only.
    Read,
    /// Create, edit.
    Write,
    /// Delete or overwrite irrecoverably.
    Destructive,
    /// View or export other people's personal data.
    SensitiveData,
    /// Touches money.
    Money,
}

impl Risk {
    /// Stable name in the API: `read`, `write`, `destructive`, `sensitive_data`, `money`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Risk::Read => "read",
            Risk::Write => "write",
            Risk::Destructive => "destructive",
            Risk::SensitiveData => "sensitive_data",
            Risk::Money => "money",
        }
    }
}

/// `text` ends with `suffix` (usable in a `const fn`).
const fn ends_with(text: &str, suffix: &str) -> bool {
    let (text, suffix) = (text.as_bytes(), suffix.as_bytes());
    if suffix.len() > text.len() {
        return false;
    }
    let offset = text.len() - suffix.len();
    let mut index = 0;
    while index < suffix.len() {
        if text[offset + index] != suffix[index] {
            return false;
        }
        index += 1;
    }
    true
}

impl Permission {
    /// `name`: `<module>.<action>`, the segments being lowercase letters, digits and `_`.
    /// `description`: an English sentence for the person granting permissions to read (shown in the API).
    ///
    /// Risk level derived from the name: `….read` is [`Risk::Read`], anything else is [`Risk::Write`];
    /// otherwise set it with [`with_risk`](Self::with_risk).
    pub const fn new(name: &'static str, description: &'static str) -> Self {
        let risk = if ends_with(name, ".read") { Risk::Read } else { Risk::Write };
        Permission { name, description, risk, scoped: false }
    }

    /// Permission that applies to data with `org_id`: a role can grant it with depth `own`, `unit`,
    /// `subtree`. A permission that is not `scoped` is granted only at depth `tenant` and takes effect across
    /// the whole tenant regardless of which node the membership is at.
    pub const fn scoped(mut self) -> Self {
        self.scoped = true;
        self
    }

    /// Whether it is [`scoped`](Self::scoped).
    pub const fn is_scoped(&self) -> bool {
        self.scoped
    }

    /// Sets the risk level, e.g. `Permission::new("notes.purge", "…").with_risk(Risk::Destructive)`.
    pub const fn with_risk(mut self, risk: Risk) -> Self {
        self.risk = risk;
        self
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn description(&self) -> &'static str {
        self.description
    }

    pub const fn risk(&self) -> Risk {
        self.risk
    }
}

/// All permissions of the program, in name order.
#[derive(Debug, Clone, Default)]
pub struct PermissionCatalog {
    // `Arc`: danh mục dựng một lần lúc khởi động rồi chia cho nhiều module đọc.
    permissions: Arc<Vec<Permission>>,
}

impl PermissionCatalog {
    /// Validates and collects the permissions of modules (`(module name, its permissions)`).
    /// Returns all problems at once if there are any.
    pub fn new(modules: Vec<(&'static str, Vec<Permission>)>) -> Result<Self, Vec<String>> {
        let mut problems = Vec::new();
        let mut seen = BTreeSet::new();
        let mut permissions = Vec::new();
        for (module, declared) in modules {
            for permission in declared {
                problems.extend(name_problem(module, permission.name));
                if permission.description.trim().is_empty() {
                    problems.push(format!(
                        "permission `{}` of module `{module}` has no description",
                        permission.name
                    ));
                }
                if !seen.insert(permission.name) {
                    problems.push(format!("permission `{}` is declared twice", permission.name));
                }
                permissions.push(permission);
            }
        }
        if !problems.is_empty() {
            return Err(problems);
        }
        permissions.sort();
        Ok(PermissionCatalog { permissions: Arc::new(permissions) })
    }

    /// Whether a permission named `name` exists.
    pub fn contains(&self, name: &str) -> bool {
        self.permissions.iter().any(|permission| permission.name == name)
    }

    /// All permissions, in name order.
    pub fn iter(&self) -> impl Iterator<Item = &Permission> {
        self.permissions.iter()
    }
}

/// Problem with the permission name `name` declared by module `module`, if any.
fn name_problem(module: &str, name: &str) -> Option<String> {
    let mut segments = name.split('.');
    let valid = segments.next() == Some(module)
        && segments.clone().next().is_some()
        && segments.all(is_identifier);
    (!valid).then(|| {
        format!(
            "permission `{name}` of module `{module}` must be named `{module}.<action>`, with \
             segments of lowercase letters, digits and `_` (e.g. `{module}.write`)"
        )
    })
}

/// The authenticated caller of the API: which user and which permissions they have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    user_id: Uuid,
    grants: Grants,
    /// ID of the API key, when the request arrives with an API key instead of a login session.
    api_key: Option<Uuid>,
    /// The caller's tenant (step 33): the default tenant unless the credential carries another tenant.
    tenant: TenantId,
    /// The platform tenant of staff entering another tenant through the `Autoxide-Tenant` header.
    operator_from: Option<TenantId>,
    /// Staff or buyer (step 34).
    realm: Realm,
    /// Fence of a node-bound API key: all permissions take effect only within this branch.
    fence: Option<OrgId>,
    /// The route into the current tenant.
    access: AccessPath,
    /// Kind of actor from the credential (API key `agent`, `service`…).
    kind: ActorKind,
    /// Permissions banned by a role (step 54, deny wins over allow): removed from `grants`; the names are
    /// kept so the error states the reason (`denied_by_role`) rather than "missing". Permissions outside the
    /// tenant's guardrail (step 54) are also removed, with the reason `outside_guardrail`.
    denied: Arc<BTreeMap<String, &'static str>>,
    /// IP ranges allowed to call (the API key declares `allowed_ips`, step 54); `None`: any address.
    networks: Option<Arc<Vec<crate::config::IpRange>>>,
    /// Re-confirmation for sensitive actions (step 54 P5); `None`: not required.
    step_up: Option<StepUp>,
    /// Units already shown to exist while checking permissions (shared by clones; never compared).
    known_units: decision::KnownUnits,
    /// The OAuth grant when the request arrived with an OAuth access token (step 71 G4).
    oauth: Option<Arc<OAuthAccess>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Grants {
    /// All permissions at depth `tenant`, including permissions added later (role `admin`).
    All,
    /// Permission → its grants (node, depth). A name being present means having the permission at at least one node.
    Only(Arc<BTreeMap<String, Vec<Grant>>>),
}

/// Snapshot of a [`Principal`] to write into a job's payload (step 51 P15–P16): the job runs later, with no
/// request, but must check permissions exactly as the person who requested it — the same grants at the same
/// nodes, the same API key fence. If permissions are revoked after the job is enqueued, the job still runs
/// with the permissions at request time (like every job queue): jobs are short and audited.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrincipalSnapshot {
    user_id: Uuid,
    /// `None`: all permissions (role `admin`).
    grants: Option<Vec<(String, Uuid, Scope)>>,
    api_key: Option<Uuid>,
    tenant: Uuid,
    operator_from: Option<Uuid>,
    realm: Realm,
    fence: Option<Uuid>,
    access: AccessPath,
    #[serde(default)]
    kind: ActorKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    denied: Vec<String>,
    /// OAuth grant and client when the caller was an OAuth client: the job keeps acting as that client.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    oauth: Option<(Uuid, String)>,
}

impl PrincipalSnapshot {
    /// Rebuilds the snapshotted caller.
    pub fn restore(&self) -> Principal {
        let grants = match &self.grants {
            None => Grants::All,
            Some(list) => {
                let mut map: BTreeMap<String, Vec<Grant>> = BTreeMap::new();
                for (permission, org, scope) in list {
                    map.entry(permission.clone())
                        .or_default()
                        .push(Grant::new(OrgId::from_uuid(*org), *scope));
                }
                Grants::Only(Arc::new(map))
            }
        };
        Principal {
            user_id: self.user_id,
            grants,
            api_key: self.api_key,
            tenant: TenantId::from_uuid(self.tenant),
            operator_from: self.operator_from.map(TenantId::from_uuid),
            realm: self.realm,
            fence: self.fence.map(OrgId::from_uuid),
            access: self.access,
            kind: self.kind,
            denied: Arc::new(
                self.denied.iter().map(|name| (name.clone(), DENIED_BY_ROLE)).collect(),
            ),
            // Job chạy không có request nên không có địa chỉ để kiểm: địa chỉ đã kiểm lúc yêu cầu.
            networks: None,
            // Job không có phiên để xác nhận lại: đã kiểm lúc yêu cầu.
            step_up: None,
            known_units: decision::KnownUnits::default(),
            // Job không qua HTTP nên không có resource để kiểm audience: chỉ giữ loại credential.
            oauth: self.oauth.as_ref().map(|(grant, client)| {
                Arc::new(OAuthAccess::new(*grant, client.clone(), String::new()))
            }),
        }
    }

    /// The person who made the request.
    pub fn user_id(&self) -> Uuid {
        self.user_id
    }
}

impl Principal {
    /// Snapshots the caller so a job running later checks permissions as them ([`PrincipalSnapshot::restore`]).
    pub fn snapshot(&self) -> PrincipalSnapshot {
        let grants = match &self.grants {
            Grants::All => None,
            Grants::Only(map) => Some(
                map.iter()
                    .flat_map(|(permission, grants)| {
                        grants
                            .iter()
                            .map(|grant| (permission.clone(), grant.org.as_uuid(), grant.scope))
                    })
                    .collect(),
            ),
        };
        PrincipalSnapshot {
            user_id: self.user_id,
            grants,
            api_key: self.api_key,
            tenant: self.tenant.as_uuid(),
            operator_from: self.operator_from.map(|tenant| tenant.as_uuid()),
            realm: self.realm,
            fence: self.fence.map(|fence| fence.as_uuid()),
            access: self.access,
            kind: self.kind,
            denied: self.denied.keys().cloned().collect(),
            oauth: self
                .oauth
                .as_ref()
                .map(|access| (access.grant_id(), access.client_id().to_owned())),
        }
    }
}

impl Principal {
    /// Sets the kind of actor (the authenticator, by API key kind).
    pub fn with_actor_kind(mut self, kind: ActorKind) -> Self {
        self.kind = kind;
        self
    }

    /// Kind of actor: `Support` when entering through a support access grant, otherwise by credential.
    pub fn actor_kind(&self) -> ActorKind {
        // Client OAuth luôn là agent, dù bộ xác thực có quên đặt loại (D-20261126-mcp-oauth).
        if self.oauth.is_some() {
            return ActorKind::Agent;
        }
        match self.access {
            AccessPath::Support(_) => ActorKind::Support,
            _ => self.kind,
        }
    }

    /// The user `user_id` with exactly the permissions `permissions`, at depth `tenant`.
    pub fn new(user_id: Uuid, permissions: impl IntoIterator<Item = String>) -> Self {
        let root = root_of(DEFAULT_TENANT);
        Principal::with_grants(
            user_id,
            permissions.into_iter().map(|permission| (permission, Grant::new(root, Scope::Tenant))),
        )
    }

    /// The user `user_id` with the grants `(permission, grant)` — one permission can have several
    /// grants at several nodes; the permission is their union.
    pub fn with_grants(user_id: Uuid, grants: impl IntoIterator<Item = (String, Grant)>) -> Self {
        let mut map: BTreeMap<String, Vec<Grant>> = BTreeMap::new();
        for (permission, grant) in grants {
            let list = map.entry(permission).or_default();
            if !list.contains(&grant) {
                list.push(grant);
            }
        }
        Principal::with(user_id, Grants::Only(Arc::new(map)))
    }

    /// The user `user_id` with every permission.
    pub fn with_all_permissions(user_id: Uuid) -> Self {
        Principal::with(user_id, Grants::All)
    }

    fn with(user_id: Uuid, grants: Grants) -> Self {
        Principal {
            user_id,
            grants,
            api_key: None,
            tenant: DEFAULT_TENANT,
            operator_from: None,
            realm: Realm::Staff,
            fence: None,
            access: AccessPath::Direct,
            kind: ActorKind::Human,
            denied: Arc::default(),
            networks: None,
            step_up: None,
            known_units: decision::KnownUnits::default(),
            oauth: None,
        }
    }

    /// The buyer `user_id` (step 34): can sign in, has no permissions.
    pub fn customer(user_id: Uuid) -> Self {
        let mut principal = Principal::with(user_id, Grants::Only(Arc::default()));
        principal.realm = Realm::Customer;
        principal
    }

    /// Staff or buyer.
    pub fn realm(&self) -> Realm {
        self.realm
    }

    /// All permissions take effect only within the branch of node `org` (node-bound API key). The tenant's
    /// root node means no fence.
    pub fn with_fence(mut self, org: OrgId) -> Self {
        self.fence = Some(org);
        self
    }

    /// The fence currently in effect; `None` when there is none (including when the fence is at the root node).
    pub fn fence(&self) -> Option<OrgId> {
        self.fence.filter(|org| *org != root_of(self.tenant))
    }

    /// Marks the route into the current tenant (delegation, support).
    pub fn with_access(mut self, access: AccessPath) -> Self {
        self.access = access;
        self
    }

    /// The route into the current tenant.
    pub fn access(&self) -> AccessPath {
        self.access
    }

    /// Keeps only the permissions in `names`, each with exactly the grants (node, depth) it currently has —
    /// e.g. an API key's permissions = the key's permissions ∩ the key owner's current permissions.
    #[must_use]
    pub fn only<'a>(mut self, names: impl IntoIterator<Item = &'a str>) -> Self {
        let root = root_of(self.tenant);
        let mut kept: BTreeMap<String, Vec<Grant>> = BTreeMap::new();
        for name in names {
            let grants = match &self.grants {
                Grants::All => vec![Grant::new(root, Scope::Tenant)],
                Grants::Only(map) => map.get(name).cloned().unwrap_or_default(),
            };
            if !grants.is_empty() {
                kept.insert(name.to_owned(), grants);
            }
        }
        self.grants = Grants::Only(Arc::new(kept));
        self
    }

    /// Removes the permissions `denied` (step 54: deny wins over allow — when one role bans a permission no
    /// other role, including a full-access role, can grant it back). A person with every permission becomes a
    /// person with every permission of `catalog` (depth `tenant`) except the banned part; permissions are
    /// reloaded on every request, so a permission added later is still present on the next request.
    #[must_use]
    pub fn without(
        self,
        denied: impl IntoIterator<Item = String>,
        catalog: &PermissionCatalog,
    ) -> Self {
        let denied: BTreeSet<String> = denied.into_iter().collect();
        self.remove(|name| denied.contains(name), DENIED_BY_ROLE, catalog)
    }

    /// Keeps only the permissions inside the tenant's guardrail (step 54, like AWS SCPs): the parent tenant /
    /// platform sets the maximum permissions for a child tenant, and no role of the child tenant — including
    /// `admin` — can exceed it. `allowed`: `None` when the tenant has no guardrail.
    #[must_use]
    pub fn within(self, allowed: Option<&BTreeSet<String>>, catalog: &PermissionCatalog) -> Self {
        let Some(allowed) = allowed else { return self };
        self.remove(|name| !allowed.contains(name), OUTSIDE_GUARDRAIL, catalog)
    }

    /// Removes every permission selected by `drop`, recording `reason` for later errors.
    fn remove(
        mut self,
        drop: impl Fn(&str) -> bool,
        reason: &'static str,
        catalog: &PermissionCatalog,
    ) -> Self {
        let root = root_of(self.tenant);
        let mut kept: BTreeMap<String, Vec<Grant>> = match &self.grants {
            Grants::All => catalog
                .iter()
                .map(|permission| {
                    (permission.name.to_owned(), vec![Grant::new(root, Scope::Tenant)])
                })
                .collect(),
            Grants::Only(map) => (**map).clone(),
        };
        let mut removed: Vec<String> = Vec::new();
        // Tên bị bỏ ghi theo danh mục: cả permission người gọi chưa từng có cũng mang lý do đúng.
        for permission in catalog.iter() {
            if drop(permission.name) {
                removed.push(permission.name.to_owned());
            }
        }
        for name in kept.keys() {
            if drop(name) && !removed.contains(name) {
                removed.push(name.clone());
            }
        }
        // Không bỏ gì: giữ nguyên, kể cả "mọi permission" (gồm cả permission thêm sau).
        if removed.is_empty() {
            return self;
        }
        kept.retain(|name, _| !drop(name));
        self.grants = Grants::Only(Arc::new(kept));
        let mut all = (*self.denied).clone();
        for name in removed {
            // Lý do đầu tiên giữ nguyên: rào áp sau role cấm, nhưng permission bị cả hai chặn
            // thì nói lý do của role (người trong khách sửa được).
            all.entry(name).or_insert(reason);
        }
        self.denied = Arc::new(all);
        self
    }

    /// Accepts requests only from the ranges `networks` (the API key declares `allowed_ips`). An empty list
    /// means no restriction.
    #[must_use]
    pub fn with_networks(mut self, networks: Vec<crate::config::IpRange>) -> Self {
        self.networks = (!networks.is_empty()).then(|| Arc::new(networks));
        self
    }

    /// `Ok` when the request from address `client` is allowed; otherwise 403 `network_not_allowed`. If the
    /// address is unknown (not over HTTP) and the credential has a range restriction, it is rejected: no
    /// guessing.
    pub fn admits(&self, client: Option<std::net::IpAddr>) -> Result<(), Error> {
        let Some(networks) = &self.networks else { return Ok(()) };
        if client.is_some_and(|ip| networks.iter().any(|range| range.contains(ip))) {
            return Ok(());
        }
        Err(Error::forbidden(
            "network_not_allowed",
            "This credential can only be used from the IP addresses it lists, and this request \
             comes from another address. Call from an allowed network, or change the key's \
             allowed addresses.",
        ))
    }

    /// The permission `name` is banned by a role or lies outside the tenant's guardrail.
    pub fn is_denied(&self, name: &str) -> bool {
        self.denied.contains_key(name)
    }

    /// Error 403 `permission_denied` for `permission`, with the `reason` `denied_by_role`,
    /// `outside_guardrail` hay `missing`.
    fn refuse(&self, permission: &Permission) -> Error {
        let reason = self.denied.get(permission.name).copied().unwrap_or("missing");
        oauth::challenge_scope(
            permission_denied(permission).with_reason(reason),
            reason,
            permission.name,
        )
    }

    /// Default node for a new record when the creator does not choose one (step 34): the root node if they
    /// have `permission` for the whole tenant; the only node if all their grants are at one node; `None` when
    /// they work at several nodes (they must choose) or do not have `permission`.
    pub fn home_org(&self, tenant: TenantId, permission: &Permission) -> Option<OrgId> {
        let grants = self.grants(permission);
        let first = grants.first()?;
        if grants.iter().any(|grant| grant.scope == Scope::Tenant) {
            return Some(root_of(tenant));
        }
        grants.iter().all(|grant| grant.org == first.org).then_some(first.org)
    }

    /// All grants by permission (name → grants); `None` when they have every permission at depth `tenant`.
    pub fn grant_map(&self) -> Option<&BTreeMap<String, Vec<Grant>>> {
        match &self.grants {
            Grants::All => None,
            Grants::Only(map) => Some(map),
        }
    }

    /// Whether the caller works at node `org`: any grant (any permission) at depth
    /// `tenant`, or `unit` / `subtree` covering that node. Used when the caller names a node for something not
    /// tied to one particular permission (e.g. a node-bound API key).
    pub async fn reaches<D: crate::db::Executor>(&self, db: &D, org: OrgId) -> Result<bool, Error> {
        let db = &crate::db::Via::of(db);
        let Grants::Only(map) = &self.grants else { return Ok(true) };
        let mut subtrees = Vec::new();
        for grant in map.values().flatten() {
            match grant.scope {
                Scope::Tenant => return Ok(true),
                Scope::Unit | Scope::Own if grant.org == org => return Ok(true),
                Scope::Subtree => subtrees.push(grant.org),
                Scope::Unit | Scope::Own => {}
            }
        }
        if subtrees.is_empty() {
            return Ok(false);
        }
        OrgTree::is_within(db, self.tenant, org, &subtrees).await
    }

    /// The grants of `permission`; empty when there are none. A permission that is not `scoped` becomes a
    /// `tenant` grant wherever it has a grant (effective across the whole tenant).
    pub fn grants(&self, permission: &Permission) -> Vec<Grant> {
        let tenant_wide = vec![Grant::new(root_of(self.tenant), Scope::Tenant)];
        match &self.grants {
            Grants::All => tenant_wide,
            Grants::Only(map) => match map.get(permission.name) {
                None => Vec::new(),
                Some(_) if !permission.is_scoped() => tenant_wide,
                Some(list) => list.clone(),
            },
        }
    }

    /// The caller belongs to tenant `tenant` (the authenticator sets it from the tenant in the credential).
    pub fn in_tenant(mut self, tenant: TenantId) -> Self {
        self.tenant = tenant;
        self
    }

    /// The request's tenant: the credential's tenant, or the tenant that platform staff are entering.
    pub fn tenant_id(&self) -> TenantId {
        self.tenant
    }

    /// The platform tenant of staff entering another tenant (`Autoxide-Tenant`); `None` when calling within
    /// their own tenant.
    pub fn operator_from(&self) -> Option<TenantId> {
        self.operator_from
    }

    /// Platform staff enter tenant `target`: permissions are kept, the request's tenant changes.
    pub fn operating(mut self, target: TenantId) -> Self {
        if target != self.tenant {
            self.operator_from = Some(self.tenant);
            self.tenant = target;
        }
        self
    }

    /// Marks that the request arrived with API key `id`.
    pub fn via_api_key(mut self, id: Uuid) -> Self {
        self.api_key = Some(id);
        self
    }

    /// ID of the API key if the request arrived with an API key.
    pub fn api_key_id(&self) -> Option<Uuid> {
        self.api_key
    }

    pub fn user_id(&self) -> Uuid {
        self.user_id
    }

    /// Whether it has every permission (including permissions that did not exist when granted).
    pub fn has_all_permissions(&self) -> bool {
        self.grants == Grants::All
    }

    /// The explicitly granted permissions (at any depth), in name order. Empty when
    /// [`has_all_permissions`](Self::has_all_permissions).
    pub fn permissions(&self) -> Vec<String> {
        match &self.grants {
            Grants::All => Vec::new(),
            Grants::Only(map) => map.keys().cloned().collect(),
        }
    }

    pub fn has(&self, permission: &Permission) -> bool {
        self.allows(permission.name)
    }

    /// Like [`has`](Self::has), by permission name (e.g. a name sent by the client). Having it at any
    /// depth counts as having it.
    pub fn allows(&self, permission: &str) -> bool {
        match &self.grants {
            Grants::All => true,
            Grants::Only(map) => map.contains_key(permission),
        }
    }

    /// `Ok` if it has `permission` (at any depth); otherwise a 403 `permission_denied` error naming
    /// it. Node-scoped data is checked further with [`require_at`](Self::require_at) or
    /// [`scope_condition`](Self::scope_condition).
    pub fn require(&self, permission: &Permission) -> Result<(), Error> {
        if self.has(permission) {
            return self.fresh_for(permission);
        }
        Err(self.refuse(permission))
    }
}

/// Reason of `permission_denied`: a role of the caller bans the permission.
const DENIED_BY_ROLE: &str = "denied_by_role";
/// Reason of `permission_denied`: the permission lies outside the guardrail the parent tenant set for this tenant.
const OUTSIDE_GUARDRAIL: &str = "outside_guardrail";

fn permission_denied(permission: &Permission) -> Error {
    Error::forbidden(
        "permission_denied",
        "This action needs the permission `{permission}`. Ask an administrator for a role that \
         has it.",
    )
    .with_param("permission", permission.name)
}

fn org_out_of_scope(org: OrgId, permission: &Permission) -> Error {
    Error::forbidden(
        "org_out_of_scope",
        "The organization unit `{org}` is outside the scope of your permission `{permission}`. \
         Choose a unit you work in, or ask an administrator for a wider scope.",
    )
    .with_param("org", org.as_uuid())
    .with_param("permission", permission.name)
}

/// Asks "has this account verified (phone number / identity) to post content" — the hook for the requirement
/// to verify accounts that post content under Decree 147/2024/ND-CP (D-202). The module that holds
/// verification state (step 37) registers it with
/// [`StartContext::posting_verifier`](crate::module::StartContext::posting_verifier); modules that accept
/// user content (e.g. `comments`) ask through [`PostingVerification`].
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait PostingVerifier: Send + Sync {
    /// Whether the user `user_id` of tenant `tenant` has verified to post content.
    async fn is_verified(&self, tenant: &TenantContext, user_id: Uuid) -> Result<bool, Error>;

    /// Tenant `tenant` requires verification at EVERY place content is posted (not only the target kinds the
    /// asking party itself configures as "verified accounts only") — e.g. a tenant sets a policy that posting
    /// comments needs a phone number. Default `false` (D-038): old verifiers need no changes.
    async fn required_everywhere(&self, tenant: &TenantContext) -> Result<bool, Error> {
        let _ = tenant;
        Ok(false)
    }
}

/// Holder of the program's single [`PostingVerifier`]. Cheap to clone: all clones see the same
/// holder, even when the registering module starts AFTER the asking module.
///
/// If no module has registered, every signed-in account counts as verified — keeping the behavior
/// before D-202; a guest (no account) is rejected by the asking party itself.
#[derive(Clone, Default)]
pub struct PostingVerification {
    // Ghi một lần lúc khởi động; đọc mỗi lần hỏi. Không giữ khoá qua `await`.
    verifier: Arc<std::sync::RwLock<Option<Registered>>>,
}

/// The registered verifier and the module that registered it.
type Registered = (&'static str, Arc<dyn PostingVerifier>);

impl std::fmt::Debug for PostingVerification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostingVerification").field("registered", &self.is_registered()).finish()
    }
}

impl PostingVerification {
    /// Registers the verifier of module `module`. Only one module may register.
    pub fn register(
        &self,
        module: &'static str,
        verifier: Arc<dyn PostingVerifier>,
    ) -> Result<(), Error> {
        let mut slot = self.verifier.write().map_err(|_| poisoned_verification())?;
        if let Some((owner, _)) = slot.as_ref() {
            return Err(Error::internal(format!(
                "module `{module}` registers an account verifier, but module `{owner}` already \
                 does; there is exactly one"
            )));
        }
        *slot = Some((module, verifier));
        Ok(())
    }

    /// Whether a module has registered a verifier yet.
    pub fn is_registered(&self) -> bool {
        self.verifier.read().is_ok_and(|slot| slot.is_some())
    }

    /// Whether the user `user_id` has verified; `true` when no module has registered a verifier.
    pub async fn is_verified(&self, tenant: &TenantContext, user_id: Uuid) -> Result<bool, Error> {
        let verifier = {
            let slot = self.verifier.read().map_err(|_| poisoned_verification())?;
            slot.as_ref().map(|(_, verifier)| verifier.clone())
        };
        match verifier {
            Some(verifier) => verifier.is_verified(tenant, user_id).await,
            None => Ok(true),
        }
    }

    /// Whether tenant `tenant` requires verification at every place content is posted; `false` when no module
    /// has registered a verifier.
    pub async fn required_everywhere(&self, tenant: &TenantContext) -> Result<bool, Error> {
        let verifier = {
            let slot = self.verifier.read().map_err(|_| poisoned_verification())?;
            slot.as_ref().map(|(_, verifier)| verifier.clone())
        };
        match verifier {
            Some(verifier) => verifier.required_everywhere(tenant).await,
            None => Ok(false),
        }
    }
}

fn poisoned_verification() -> Error {
    Error::internal("the account verification slot is unusable after a panic")
}

// Handler viết `principal: Principal` thì axum gọi hàm này trước khi chạy handler:
// request không xác thực được bị trả lỗi ngay, handler không bao giờ chạy.
impl<S: Send + Sync> FromRequestParts<S> for Principal {
    type Rejection = Error;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let authenticator =
            parts.extensions.get::<Arc<dyn Authenticator>>().cloned().ok_or_else(|| {
                Error::internal(
                    "an endpoint needs a signed-in user, but no module provides \
                     `dyn Authenticator`; register the `auth` module",
                )
            })?;
        // Xác thực và kiểm khách của credential với tên miền / header (bước 33). `Box::pin`: future
        // này lồng nhiều tầng (bộ xác thực, sổ database); đóng hộp để future của mọi handler nhận
        // `Principal` không phình theo, tránh vượt giới hạn độ sâu của trình biên dịch.
        Box::pin(crate::tenancy::request::authenticate(&authenticator, parts)).await
    }
}

// Handler viết `principal: Option<Principal>`: endpoint công khai. Không có header
// `Authorization` thì `None` (kể cả khi chưa module nào xác thực); có thì xác thực như
// trên, sai là lỗi.
impl<S: Send + Sync> OptionalFromRequestParts<S> for Principal {
    type Rejection = Error;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &S,
    ) -> Result<Option<Self>, Self::Rejection> {
        if !parts.headers.contains_key(header::AUTHORIZATION) {
            return Ok(None);
        }
        <Principal as FromRequestParts<S>>::from_request_parts(parts, state).await.map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;

    #[test]
    fn ban_chup_nguoi_goi_dung_lai_dung_quyen() {
        let org = OrgId::from_uuid(Uuid::now_v7());
        let principal = Principal::with_grants(
            Uuid::now_v7(),
            [
                ("content.read".to_owned(), Grant::new(org, Scope::Subtree)),
                ("content.write".to_owned(), Grant::new(org, Scope::Own)),
            ],
        )
        .via_api_key(Uuid::now_v7());
        let snapshot: PrincipalSnapshot =
            serde_json::from_value(serde_json::to_value(principal.snapshot()).unwrap()).unwrap();
        let restored = snapshot.restore();
        assert_eq!(restored.user_id(), principal.user_id());
        assert_eq!(restored.grant_map(), principal.grant_map());
        assert_eq!(restored.snapshot(), principal.snapshot());
        let admin = Principal::with_all_permissions(Uuid::now_v7());
        assert!(admin.snapshot().restore().has_all_permissions());
    }

    const READ: Permission = Permission::new("notes.read", "Read notes.");

    struct OnlyOne(Uuid);

    #[async_trait::async_trait]
    impl PostingVerifier for OnlyOne {
        async fn is_verified(
            &self,
            _tenant: &crate::tenancy::TenantContext,
            user_id: Uuid,
        ) -> Result<bool, Error> {
            Ok(user_id == self.0)
        }
    }

    #[tokio::test]
    async fn xac_thuc_tai_khoan_mac_dinh_cho_qua_khi_chua_ai_dang_ky() {
        let slot = PostingVerification::default();
        let early = slot.clone();
        let (verified, other) = (Uuid::now_v7(), Uuid::now_v7());
        assert!(!slot.is_registered());
        assert!(
            slot.is_verified(&crate::tenancy::test_tenant(), other).await.unwrap(),
            "no verifier: as before D-202"
        );
        slot.register("auth", Arc::new(OnlyOne(verified))).unwrap();
        assert!(slot.register("compliance", Arc::new(OnlyOne(verified))).is_err(), "one only");
        assert!(early.is_registered(), "clones see a later registration");
        assert!(early.is_verified(&crate::tenancy::test_tenant(), verified).await.unwrap());
        assert!(!early.is_verified(&crate::tenancy::test_tenant(), other).await.unwrap());
    }
    const WRITE: Permission = Permission::new("notes.write", "Write notes.");

    #[test]
    fn danh_muc_hop_le_sap_theo_ten() {
        let catalog = PermissionCatalog::new(vec![
            ("notes", vec![WRITE, READ]),
            ("auth", vec![Permission::new("auth.roles.manage", "Manage roles.")]),
        ])
        .unwrap();

        let names: Vec<&str> = catalog.iter().map(Permission::name).collect();
        assert_eq!(names, ["auth.roles.manage", "notes.read", "notes.write"]);
        assert!(catalog.contains("notes.read"));
        assert!(!catalog.contains("notes.delete"));
    }

    #[test]
    fn ten_sai_trung_hoac_thieu_mo_ta_bi_bao_cung_luc() {
        let problems = PermissionCatalog::new(vec![(
            "notes",
            vec![
                Permission::new("notes", "No action."),
                Permission::new("tags.read", "Wrong module."),
                Permission::new("notes.Write", "Upper case."),
                Permission::new("notes..read", "Empty segment."),
                Permission::new("notes.read", "First."),
                Permission::new("notes.read", " "),
            ],
        )])
        .unwrap_err();

        assert_eq!(problems.len(), 6, "{problems:#?}");
        assert!(problems[0].contains("must be named `notes.<action>`"), "{problems:#?}");
        assert!(problems.iter().any(|problem| problem.contains("declared twice")));
        assert!(problems.iter().any(|problem| problem.contains("has no description")));
    }

    #[test]
    fn principal_kiem_permission() {
        let user = Uuid::now_v7();
        let reader = Principal::new(user, ["notes.read".to_owned()]);
        let admin = Principal::with_all_permissions(user);

        assert!(reader.require(&READ).is_ok());
        let denied = reader.require(&WRITE).unwrap_err();
        assert_eq!(denied.kind(), ErrorKind::Forbidden);
        assert_eq!(denied.code(), "permission_denied");
        assert_eq!(
            denied.message(),
            "This action needs the permission `notes.write`. Ask an administrator for a role \
             that has it."
        );
        assert!(admin.require(&WRITE).is_ok());
        assert!(admin.has_all_permissions());
        assert_eq!(reader.permissions(), ["notes.read"]);
        assert!(reader.allows("notes.read") && !reader.allows("notes.write"));
        assert_eq!(reader.api_key_id(), None);
        let key = Uuid::now_v7();
        assert_eq!(reader.clone().via_api_key(key).api_key_id(), Some(key));
        assert_eq!(reader.user_id(), user);
    }
}
