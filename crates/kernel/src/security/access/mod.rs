//! Quyền truy cập: permission do module khai, và "ai đang gọi" ([`Principal`]).
//!
//! - Mỗi module khai các permission của mình qua `Module::permissions`. Tên viết
//!   `<module>.<việc>` (vd `notes.write`, `auth.roles.manage`); core kiểm tên lúc
//!   khởi động và gom thành một danh mục ([`PermissionCatalog`]).
//! - Một module (thường là `auth`) công bố trait [`Authenticator`]: đọc header
//!   `Authorization` và trả về [`Principal`]. Kernel gắn nó vào mọi request.
//! - Handler cần đăng nhập nhận `principal: Principal` làm tham số, rồi gọi
//!   `principal.require(&NOTES_WRITE)?`. Thiếu quyền thì trả 403 `permission_denied`.
//! - Endpoint công khai (ai cũng gọi được, người đăng nhập thấy nhiều hơn) nhận
//!   `principal: Option<Principal>`: không gửi `Authorization` thì `None`; gửi mà sai
//!   thì vẫn 401 như endpoint thường — không lặng lẽ coi như khách. Tài liệu OpenAPI của
//!   handler đó khai `security((), ("bearer" = []))`.
//!
//! Nhờ vậy module nghiệp vụ kiểm quyền mà không phụ thuộc module `auth`, và module
//! `auth` có thể thay bằng cách xác thực khác mà không module nào phải sửa.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::sync::Arc;

use axum::extract::{FromRequestParts, OptionalFromRequestParts};
use axum::http::header;
use axum::http::request::Parts;
use sea_orm::ConnectionTrait;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

mod decision;

pub use decision::Decision;

use crate::error::Error;
use crate::module::is_identifier;
use crate::tenancy::TenantContext;
use crate::tenancy::{DEFAULT_TENANT, OrgId, OrgTree, TenantHint, TenantId, root_of};

/// Một permission do module khai.
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
    /// Áp lên dữ liệu có `org_id` (bước 34): role cấp được với độ sâu hẹp hơn `tenant`.
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
    /// Tên ổn định trong API và database: `own`, `unit`, `subtree`, `tenant`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Scope::Own => "own",
            Scope::Unit => "unit",
            Scope::Subtree => "subtree",
            Scope::Tenant => "tenant",
        }
    }

    /// Đọc từ tên ổn định; tên lạ → `None`.
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

/// Một quyền cụ thể: ở nút `org`, độ sâu `scope`.
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

/// Đích của một lần kiểm quyền trên một bản ghi: nút sở hữu bản ghi và người sở hữu (nếu bản ghi
/// có khái niệm người sở hữu — độ sâu `own` cần nó; không có thì `own` hiểu như `unit`).
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

/// Loại bên làm (bước 51 P12, D-224): ghi vào audit (`actor_kind`), dùng để buộc duyệt lệnh rủi ro
/// khi agent gọi. Người dùng đăng nhập là `Human` (giá trị `user` trong audit, giữ như trước).
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
    /// Giá trị cột `actor_kind` của event / audit. `Human` là `user` (giá trị có từ trước bước 51).
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

    /// Ngược của [`as_str`](Self::as_str); `human` cũng là `Human`.
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

/// Đường bên gọi vào khách hiện tại (bước 34): membership trực tiếp, uỷ quyền từ khách cha, hay
/// quyền vào hỗ trợ của nhân sự nền tảng. Ghi vào event (`Actor::access`) để audit thấy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum AccessPath {
    #[default]
    Direct,
    /// Qua uỷ quyền có ID này.
    Delegation(Uuid),
    /// Qua lần cấp quyền vào hỗ trợ có ID này.
    Support(Uuid),
}

impl AccessPath {
    /// Tên ổn định: `direct`, `delegation`, `support`.
    pub const fn as_str(self) -> &'static str {
        match self {
            AccessPath::Direct => "direct",
            AccessPath::Delegation(_) => "delegation",
            AccessPath::Support(_) => "support",
        }
    }

    /// ID của uỷ quyền / lần cấp hỗ trợ; `None` khi trực tiếp.
    pub const fn id(self) -> Option<Uuid> {
        match self {
            AccessPath::Direct => None,
            AccessPath::Delegation(id) | AccessPath::Support(id) => Some(id),
        }
    }
}

/// Mức rủi ro của một permission, đọc được bằng máy (D-205, đặc tả 54 P1): giao diện và AI agent
/// biết trước việc nào chỉ đọc, việc nào cần cẩn thận; tool MCP chỉ đọc chỉ được đòi permission
/// mức [`Risk::Read`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Risk {
    /// Chỉ xem.
    Read,
    /// Tạo, sửa.
    Write,
    /// Xoá hay ghi đè không lấy lại được.
    Destructive,
    /// Xem hay xuất dữ liệu cá nhân của người khác.
    SensitiveData,
    /// Chạm tới tiền.
    Money,
}

impl Risk {
    /// Tên ổn định trong API: `read`, `write`, `destructive`, `sensitive_data`, `money`.
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

/// `text` kết thúc bằng `suffix` (dùng được trong `const fn`).
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
    /// `name`: `<module>.<việc>`, các đoạn là chữ thường, số và `_`.
    /// `description`: câu tiếng Anh cho người cấp quyền đọc (hiện trong API).
    ///
    /// Mức rủi ro suy từ tên: `….read` là [`Risk::Read`], còn lại [`Risk::Write`]; khác thì đặt bằng
    /// [`with_risk`](Self::with_risk).
    pub const fn new(name: &'static str, description: &'static str) -> Self {
        let risk = if ends_with(name, ".read") { Risk::Read } else { Risk::Write };
        Permission { name, description, risk, scoped: false }
    }

    /// Permission áp lên dữ liệu có `org_id` (bước 34): role cấp được với độ sâu `own`, `unit`,
    /// `subtree`. Permission không `scoped` chỉ cấp ở độ sâu `tenant` và có hiệu lực toàn khách
    /// dù membership ở nút nào.
    pub const fn scoped(mut self) -> Self {
        self.scoped = true;
        self
    }

    /// Có [`scoped`](Self::scoped) không.
    pub const fn is_scoped(&self) -> bool {
        self.scoped
    }

    /// Đặt mức rủi ro, vd `Permission::new("notes.purge", "…").with_risk(Risk::Destructive)`.
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

/// Mọi permission của chương trình, theo thứ tự tên.
#[derive(Debug, Clone, Default)]
pub struct PermissionCatalog {
    // `Arc`: danh mục dựng một lần lúc khởi động rồi chia cho nhiều module đọc.
    permissions: Arc<Vec<Permission>>,
}

impl PermissionCatalog {
    /// Kiểm và gom permission của các module (`(tên module, permission của nó)`).
    /// Trả về mọi vấn đề cùng lúc nếu có.
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

    /// Có permission tên `name` không.
    pub fn contains(&self, name: &str) -> bool {
        self.permissions.iter().any(|permission| permission.name == name)
    }

    /// Mọi permission, theo thứ tự tên.
    pub fn iter(&self) -> impl Iterator<Item = &Permission> {
        self.permissions.iter()
    }
}

/// Vấn đề của tên permission `name` khai bởi module `module`, nếu có.
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

/// Bên đang gọi API, đã xác thực: người dùng nào và có những permission nào.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    user_id: Uuid,
    grants: Grants,
    /// ID của API key, khi request đến bằng API key thay vì phiên đăng nhập.
    api_key: Option<Uuid>,
    /// Khách của bên gọi (bước 33): khách mặc định trừ khi credential mang khách khác.
    tenant: TenantId,
    /// Khách nền tảng của nhân sự đang vào khách khác qua header `Autoxide-Tenant`.
    operator_from: Option<TenantId>,
    /// Nhân sự hay người mua (bước 34).
    realm: Realm,
    /// Hàng rào của API key gắn nút: mọi quyền chỉ có hiệu lực trong nhánh này.
    fence: Option<OrgId>,
    /// Đường vào khách hiện tại.
    access: AccessPath,
    /// Loại bên làm theo credential (API key `agent`, `service`…).
    kind: ActorKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Grants {
    /// Mọi permission ở độ sâu `tenant`, kể cả permission thêm sau (role `admin`).
    All,
    /// Permission → các grant (nút, độ sâu). Tên có mặt nghĩa là có permission ở ít nhất một nút.
    Only(Arc<BTreeMap<String, Vec<Grant>>>),
}

/// Bản chụp của một [`Principal`] để ghi vào payload của job (bước 51 P15–P16): job chạy sau,
/// không có request, nhưng phải kiểm quyền đúng như người đã yêu cầu — cùng grant ở cùng nút, cùng
/// hàng rào của API key. Quyền bị thu hồi sau khi xếp job thì job vẫn chạy với quyền lúc yêu cầu
/// (như mọi hàng đợi job): job ngắn và có audit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrincipalSnapshot {
    user_id: Uuid,
    /// `None`: mọi permission (role `admin`).
    grants: Option<Vec<(String, Uuid, Scope)>>,
    api_key: Option<Uuid>,
    tenant: Uuid,
    operator_from: Option<Uuid>,
    realm: Realm,
    fence: Option<Uuid>,
    access: AccessPath,
    #[serde(default)]
    kind: ActorKind,
}

impl PrincipalSnapshot {
    /// Dựng lại người gọi đã chụp.
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
        }
    }

    /// Người đã yêu cầu.
    pub fn user_id(&self) -> Uuid {
        self.user_id
    }
}

impl Principal {
    /// Chụp người gọi để job chạy sau kiểm quyền như họ ([`PrincipalSnapshot::restore`]).
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
        }
    }
}

impl Principal {
    /// Đặt loại bên làm (bộ xác thực, theo loại API key).
    pub fn with_actor_kind(mut self, kind: ActorKind) -> Self {
        self.kind = kind;
        self
    }

    /// Loại bên làm: `Support` khi vào bằng quyền vào hỗ trợ, không thì theo credential.
    pub fn actor_kind(&self) -> ActorKind {
        match self.access {
            AccessPath::Support(_) => ActorKind::Support,
            _ => self.kind,
        }
    }

    /// Người dùng `user_id` với đúng các permission `permissions`, ở độ sâu `tenant`.
    pub fn new(user_id: Uuid, permissions: impl IntoIterator<Item = String>) -> Self {
        let root = root_of(DEFAULT_TENANT);
        Principal::with_grants(
            user_id,
            permissions.into_iter().map(|permission| (permission, Grant::new(root, Scope::Tenant))),
        )
    }

    /// Người dùng `user_id` với các grant `(permission, grant)` — một permission có thể có nhiều
    /// grant ở nhiều nút; quyền là hợp của chúng.
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

    /// Người dùng `user_id` có mọi permission.
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
        }
    }

    /// Người mua `user_id` (bước 34): đăng nhập được, không có permission nào.
    pub fn customer(user_id: Uuid) -> Self {
        let mut principal = Principal::with(user_id, Grants::Only(Arc::default()));
        principal.realm = Realm::Customer;
        principal
    }

    /// Nhân sự hay người mua.
    pub fn realm(&self) -> Realm {
        self.realm
    }

    /// Mọi quyền chỉ có hiệu lực trong nhánh của nút `org` (API key gắn nút). Nút gốc của khách
    /// nghĩa là không rào.
    pub fn with_fence(mut self, org: OrgId) -> Self {
        self.fence = Some(org);
        self
    }

    /// Hàng rào đang có hiệu lực; `None` khi không rào (kể cả khi rào ở nút gốc).
    pub fn fence(&self) -> Option<OrgId> {
        self.fence.filter(|org| *org != root_of(self.tenant))
    }

    /// Đánh dấu đường vào khách hiện tại (uỷ quyền, hỗ trợ).
    pub fn with_access(mut self, access: AccessPath) -> Self {
        self.access = access;
        self
    }

    /// Đường vào khách hiện tại.
    pub fn access(&self) -> AccessPath {
        self.access
    }

    /// Chỉ giữ các permission trong `names`, mỗi cái với đúng grant (nút, độ sâu) đang có — vd
    /// quyền của API key = permission của key ∩ quyền hiện tại của chủ key.
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

    /// Nút mặc định cho bản ghi mới khi người tạo không chọn (bước 34): nút gốc nếu họ có
    /// `permission` cho cả khách; nút duy nhất nếu mọi grant của nó ở một nút; `None` khi họ làm ở
    /// nhiều nút (phải chọn) hay không có `permission`.
    pub fn home_org(&self, tenant: TenantId, permission: &Permission) -> Option<OrgId> {
        let grants = self.grants(permission);
        let first = grants.first()?;
        if grants.iter().any(|grant| grant.scope == Scope::Tenant) {
            return Some(root_of(tenant));
        }
        grants.iter().all(|grant| grant.org == first.org).then_some(first.org)
    }

    /// Mọi grant theo permission (tên → grant); `None` khi có mọi permission ở độ sâu `tenant`.
    pub fn grant_map(&self) -> Option<&BTreeMap<String, Vec<Grant>>> {
        match &self.grants {
            Grants::All => None,
            Grants::Only(map) => Some(map),
        }
    }

    /// Bên gọi có làm việc ở nút `org` không: một grant bất kỳ (permission bất kỳ) ở độ sâu
    /// `tenant`, hay `unit` / `subtree` phủ nút đó. Dùng khi bên gọi tự nêu một nút cho thứ không
    /// gắn với một permission riêng (vd API key gắn nút).
    pub async fn reaches<D: ConnectionTrait>(&self, db: &D, org: OrgId) -> Result<bool, Error> {
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

    /// Các grant của `permission`; rỗng khi không có. Permission không `scoped` có grant ở đâu
    /// cũng thành một grant `tenant` (có hiệu lực toàn khách).
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

    /// Bên gọi thuộc khách `tenant` (bộ xác thực đặt theo khách trong credential).
    pub fn in_tenant(mut self, tenant: TenantId) -> Self {
        self.tenant = tenant;
        self
    }

    /// Khách của request: khách của credential, hay khách nhân sự nền tảng đang vào.
    pub fn tenant_id(&self) -> TenantId {
        self.tenant
    }

    /// Khách nền tảng của nhân sự đang vào khách khác (`Autoxide-Tenant`); `None` khi gọi trong
    /// chính khách của mình.
    pub fn operator_from(&self) -> Option<TenantId> {
        self.operator_from
    }

    /// Nhân sự nền tảng vào khách `target`: quyền giữ nguyên, khách của request đổi.
    pub fn operating(mut self, target: TenantId) -> Self {
        if target != self.tenant {
            self.operator_from = Some(self.tenant);
            self.tenant = target;
        }
        self
    }

    /// Đánh dấu request đến bằng API key `id`.
    pub fn via_api_key(mut self, id: Uuid) -> Self {
        self.api_key = Some(id);
        self
    }

    /// ID của API key nếu request đến bằng API key.
    pub fn api_key_id(&self) -> Option<Uuid> {
        self.api_key
    }

    pub fn user_id(&self) -> Uuid {
        self.user_id
    }

    /// Có mọi permission (kể cả permission chưa tồn tại lúc cấp) không.
    pub fn has_all_permissions(&self) -> bool {
        self.grants == Grants::All
    }

    /// Các permission được cấp rõ ràng (ở bất kỳ độ sâu nào), theo thứ tự tên. Rỗng khi
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

    /// Như [`has`](Self::has), theo tên permission (vd tên do client gửi lên). Có ở bất kỳ độ
    /// sâu nào là có.
    pub fn allows(&self, permission: &str) -> bool {
        match &self.grants {
            Grants::All => true,
            Grants::Only(map) => map.contains_key(permission),
        }
    }

    /// `Ok` nếu có `permission` (ở bất kỳ độ sâu nào); không thì lỗi 403 `permission_denied` nêu
    /// tên nó. Dữ liệu theo nút kiểm thêm bằng [`require_at`](Self::require_at) hay
    /// [`scope_condition`](Self::scope_condition).
    pub fn require(&self, permission: &Permission) -> Result<(), Error> {
        if self.has(permission) {
            return Ok(());
        }
        Err(permission_denied(permission))
    }
}

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

/// Xác thực request: đọc giá trị header `Authorization` và trả về bên đang gọi.
/// Một module công bố trait này lúc khởi động
/// (`context.provide::<dyn Authenticator>(…)`); kernel gắn nó vào mọi request.
//
// `#[async_trait]`: Rust chưa cho gọi hàm `async` của trait qua `dyn Trait`.
#[async_trait::async_trait]
pub trait Authenticator: Send + Sync {
    /// `authorization` là nguyên giá trị header (`None` khi không có); `hint` là khách mà request
    /// nhắm tới (tên miền, header `Autoxide-Tenant`) — credential cấp trước bước 33 không mang khách,
    /// bộ xác thực dùng nó để biết tìm ở database nào. Lỗi trả về thường là 401 (thiếu, sai, hết
    /// hạn) hoặc 403 (tài khoản bị khoá).
    async fn authenticate(
        &self,
        authorization: Option<&str>,
        hint: &TenantHint,
    ) -> Result<Principal, Error>;

    /// Người đã xác thực ở khách nhà (`principal.tenant_id()`) xin vào khách `target` (header
    /// `Autoxide-Tenant`, bước 34): uỷ quyền từ khách cha, quyền vào hỗ trợ có thời hạn. Trả
    /// `Principal` mới ở khách `target` (quyền theo đường vào, [`Principal::with_access`]) hay
    /// `None` khi không có đường vào nào. Tính lại mỗi request: thu hồi, hết hạn có hiệu lực
    /// ngay request kế. Mặc định: không có đường vào.
    async fn enter(
        &self,
        principal: &Principal,
        target: &TenantContext,
    ) -> Result<Option<Principal>, Error> {
        let _ = (principal, target);
        Ok(None)
    }

    /// `true`: chỉ [`enter`](Self::enter) quyết ai vào khách khác — nhân sự nền tảng có
    /// `kernel.tenants.operate` cũng phải có lần cấp quyền vào hỗ trợ (D-190, D-313). Mặc định
    /// `false`: luật nhân sự nền tảng của kernel (bước 33) còn áp.
    fn decides_entry(&self) -> bool {
        false
    }

    /// Quyền hiện tại của người dùng `user_id` ở khách `tenant`, không qua request (bước 51 P10,
    /// P11): kiểm người được giao việc có quyền của nguồn, liệt kê người cấp được quyền. `None`:
    /// không có người dùng đang hoạt động đó (hay bộ xác thực không biết). Mặc định: `None`.
    async fn principal_of(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
    ) -> Result<Option<Principal>, Error> {
        let _ = (tenant, user_id);
        Ok(None)
    }
}

/// Hỏi "tài khoản này đã xác thực (số điện thoại / định danh) để đăng nội dung chưa" — điểm nối
/// cho yêu cầu xác thực tài khoản đăng nội dung của Nghị định 147/2024/NĐ-CP (D-202). Module giữ
/// trạng thái xác thực (bước 37) đăng ký bằng
/// [`StartContext::posting_verifier`](crate::module::StartContext::posting_verifier); module nhận
/// nội dung của người dùng (vd `comments`) hỏi qua [`PostingVerification`].
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait PostingVerifier: Send + Sync {
    /// Người dùng `user_id` của khách `tenant` đã xác thực để đăng nội dung chưa.
    async fn is_verified(&self, tenant: &TenantContext, user_id: Uuid) -> Result<bool, Error>;

    /// Khách `tenant` đòi xác thực ở MỌI chỗ đăng nội dung (không chỉ các loại đích bên hỏi tự cấu
    /// hình là "chỉ tài khoản đã xác thực") — vd khách đặt chính sách đăng bình luận cần số điện
    /// thoại. Mặc định `false` (D-038): bộ hỏi cũ không phải sửa.
    async fn required_everywhere(&self, tenant: &TenantContext) -> Result<bool, Error> {
        let _ = tenant;
        Ok(false)
    }
}

/// Chỗ giữ bộ [`PostingVerifier`] duy nhất của chương trình. Clone rẻ: mọi bản clone thấy cùng
/// một chỗ, kể cả khi module đăng ký khởi động SAU module hỏi.
///
/// Chưa module nào đăng ký thì mọi tài khoản đăng nhập được tính là đã xác thực — giữ hành vi
/// trước D-202; khách (không tài khoản) thì bên hỏi tự từ chối.
#[derive(Clone, Default)]
pub struct PostingVerification {
    // Ghi một lần lúc khởi động; đọc mỗi lần hỏi. Không giữ khoá qua `await`.
    verifier: Arc<std::sync::RwLock<Option<Registered>>>,
}

/// Bộ hỏi đã đăng ký và module đăng ký nó.
type Registered = (&'static str, Arc<dyn PostingVerifier>);

impl std::fmt::Debug for PostingVerification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostingVerification").field("registered", &self.is_registered()).finish()
    }
}

impl PostingVerification {
    /// Đăng ký bộ hỏi của module `module`. Chỉ một module được đăng ký.
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

    /// Đã có module đăng ký bộ hỏi chưa.
    pub fn is_registered(&self) -> bool {
        self.verifier.read().is_ok_and(|slot| slot.is_some())
    }

    /// Người dùng `user_id` đã xác thực chưa; `true` khi chưa module nào đăng ký bộ hỏi.
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

    /// Khách `tenant` có đòi xác thực ở mọi chỗ đăng nội dung không; `false` khi chưa module nào
    /// đăng ký bộ hỏi.
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

    use super::*;
    use crate::error::ErrorKind;

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
