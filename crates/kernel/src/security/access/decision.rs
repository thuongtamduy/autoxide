//! Permission decision on a record: ONE description of the rule for both uses (step 54 item 1,
//! D-20261118-step54-one-decision).
//!
//! A list is filtered with an SQL condition ([`Principal::scope_condition`]); one record is checked with
//! [`Principal::check`] / [`Principal::require_at`]. These two paths used to write the rule twice, so they
//! could disagree: a list showed a record that then returned 403 when opened, or the reverse. Now both read
//! the same [`Reach`] (the part that `permission` allows, derived once from the caller's grants), and the
//! `access_check` test (randomly generated trees, grants and records) proves both paths give the same result
//! on SQLite and Postgres.

use sea_orm::sea_query::{Condition, Expr, ExprTrait as _};
use sea_orm::{ColumnTrait, ConnectionTrait};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use super::{Grants, OrgTarget, Permission, Principal, Scope, org_out_of_scope};
use crate::error::Error;
use crate::tenancy::{OrgId, OrgTree, TenantId, root_of};

/// Outcome of one permission decision on a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Allowed.
    Allow,
    /// No `permission` at any node (403 `permission_denied`).
    Missing,
    /// Has `permission` but the record is outside the scope (403 `org_out_of_scope`; for reads the service
    /// returns 404 so the record's existence is not revealed).
    OutOfScope,
}

/// Units shown to exist in a tenant (`OrgTree::contains`) while checking this caller's permissions, so a request
/// that checks many permissions at the same unit (navigation, work queue, find, summary) looks the unit up
/// once. Shared by the clones of one `Principal`, so it lives for one request or one job run; only "exists" is
/// remembered. Ordinary work never deletes a unit — only moving a tenant's branch out (split / merge of tenants)
/// removes the source tenant's tree, and a request or job running at that moment may still see the moved unit
/// until it ends (D-20261126-known-units-memo records the trade-off). Not part of the caller's
/// identity: never compared.
#[derive(Clone, Default)]
pub(super) struct KnownUnits(Arc<Mutex<HashSet<(TenantId, OrgId)>>>);

// Giữ bộ nhớ có hạn cho principal sống lâu (job): quá trần thì không nhớ thêm, chỉ tra lại.
const MAX_KNOWN_UNITS: usize = 256;

impl KnownUnits {
    fn contains(&self, tenant: TenantId, org: OrgId) -> bool {
        // `lock()` chỉ lỗi khi một thread khác panic lúc giữ khoá: khi đó coi như chưa nhớ, tra lại.
        self.0.lock().is_ok_and(|known| known.contains(&(tenant, org)))
    }

    fn remember(&self, tenant: TenantId, org: OrgId) {
        if let Ok(mut known) = self.0.lock()
            && known.len() < MAX_KNOWN_UNITS
        {
            known.insert((tenant, org));
        }
    }
}

// Hai principal giống nhau dù đã nhớ những đơn vị khác nhau: bộ nhớ không phải danh tính.
impl PartialEq for KnownUnits {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for KnownUnits {}

impl std::fmt::Debug for KnownUnits {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("KnownUnits")
    }
}

/// The part a permission allows: derived from the caller's grants, shared by the SQL condition and the
/// per-record check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Reach {
    /// Has depth `tenant`: every node.
    everywhere: bool,
    /// Depth `unit`: exactly these nodes.
    units: Vec<OrgId>,
    /// Depth `subtree`: these nodes and all their descendants.
    subtrees: Vec<OrgId>,
    /// Depth `own`: the caller's own records; a record with no owner behaves like `unit` at these nodes.
    owns: Vec<OrgId>,
    /// Fence of an API key attached to a node: every grant is effective only within this branch.
    fence: Option<OrgId>,
    tenant: TenantId,
    user: Uuid,
    known: KnownUnits,
}

impl Reach {
    /// SQL condition: the record has its node in column `org` and its owner in column `owner` (if any).
    pub(super) fn condition<C: ColumnTrait>(&self, org: C, owner: Option<C>) -> Condition {
        let column = |c: C| Expr::col((c.entity_name(), c));
        self.condition_on(column(org), owner.map(column))
    }

    /// [`condition`](Self::condition) on columns given as expressions (shared with the `db` layer).
    pub(super) fn condition_on(&self, org: Expr, owner: Option<Expr>) -> Condition {
        let mut condition = Condition::all();
        if !self.everywhere {
            let mut any = Condition::any();
            if !self.subtrees.is_empty() {
                any = any.add(OrgTree::subtree_condition_on(
                    self.tenant,
                    &self.subtrees,
                    org.clone(),
                ));
            }
            let mut units = self.units.clone();
            match owner {
                Some(owner) if !self.owns.is_empty() => any = any.add(owner.eq(self.user)),
                _ => units.extend(self.owns.iter().copied()),
            }
            if !units.is_empty() {
                any = any.add(org.clone().is_in(units.iter().map(OrgId::as_uuid)));
            }
            condition = condition.add(any);
        }
        if let Some(fence) = self.fence {
            condition = condition.add(OrgTree::subtree_condition_on(self.tenant, &[fence], org));
        }
        condition
    }

    /// Whether record `target` is inside this part, with the same meaning as [`condition`](Self::condition).
    pub(super) async fn covers<D: ConnectionTrait>(
        &self,
        db: &D,
        target: OrgTarget,
    ) -> Result<bool, Error> {
        let owned = match target.owner {
            Some(owner) => !self.owns.is_empty() && owner == self.user,
            None => self.owns.contains(&target.org),
        };
        // `known`: đơn vị đích đã được chứng minh là một nút của khách (nằm trong grant, hay trong nhánh
        // qua bảng closure). Quyền toàn khách, hay `own` theo chủ bản ghi, không nói gì về đơn vị — một ID
        // ngẫu nhiên hay của khách khác vẫn qua (PT-8): khi đó phải tra cây.
        let mut known = self.units.contains(&target.org) || self.owns.contains(&target.org);
        let mut allowed = self.everywhere || owned || self.units.contains(&target.org);
        if !allowed && !self.subtrees.is_empty() {
            allowed = OrgTree::is_within(db, self.tenant, target.org, &self.subtrees).await?;
            known = allowed;
        }
        if allowed && let Some(fence) = self.fence {
            allowed = OrgTree::is_within(db, self.tenant, target.org, &[fence]).await?;
            known = allowed;
        }
        if allowed && !known && !self.known.contains(self.tenant, target.org) {
            allowed = OrgTree::contains(db, self.tenant, target.org).await?;
            if allowed {
                self.known.remember(self.tenant, target.org);
            }
        }
        Ok(allowed)
    }
}

impl Principal {
    /// The part `permission` allows; `None` when there is no grant.
    pub(super) fn reach(&self, permission: &Permission) -> Option<Reach> {
        let grants = self.grants(permission);
        if grants.is_empty() {
            return None;
        }
        let pick = |scope: Scope| -> Vec<OrgId> {
            grants.iter().filter(|grant| grant.scope == scope).map(|grant| grant.org).collect()
        };
        Some(Reach {
            everywhere: grants.iter().any(|grant| grant.scope == Scope::Tenant),
            units: pick(Scope::Unit),
            subtrees: pick(Scope::Subtree),
            owns: pick(Scope::Own),
            fence: self.fence(),
            tenant: self.tenant_id(),
            user: self.user_id(),
            known: self.known_units.clone(),
        })
    }

    /// The single decision function for one record: whether the caller may use `permission` on `target`.
    /// [`require_at`](Self::require_at) turns the result into an error; lists use
    /// [`scope_condition`](Self::scope_condition), generated from the same rule.
    pub async fn check<D: ConnectionTrait>(
        &self,
        db: &D,
        permission: &Permission,
        target: OrgTarget,
    ) -> Result<Decision, Error> {
        let Some(reach) = self.reach(permission) else { return Ok(Decision::Missing) };
        self.fresh_for(permission)?;
        Ok(if reach.covers(db, target).await? { Decision::Allow } else { Decision::OutOfScope })
    }

    /// SeaORM condition that restricts a list to the part `permission` allows: column `org` is the record's
    /// node, `owner` is the owner column (if any; without it `own` is read as `unit`).
    /// `Condition::all()` (no restriction) when depth `tenant` is held and there is no fence; the error
    /// `permission_denied` when there is no grant. Use it together with `.scoped(&tenant)`.
    pub fn scope_condition<C: ColumnTrait>(
        &self,
        permission: &Permission,
        org: C,
        owner: Option<C>,
    ) -> Result<Condition, Error> {
        let reach = self.reach(permission).ok_or_else(|| self.refuse(permission))?;
        self.fresh_for(permission)?;
        Ok(reach.condition(org, owner))
    }

    /// [`scope_condition`](Self::scope_condition) for the `db` layer: the rows of `R` that `permission` lets
    /// the caller see, by node column `org` and (optionally) owner column `owner`.
    pub fn scope<R, T, U>(
        &self,
        permission: &Permission,
        org: crate::db::Col<R, T>,
        owner: Option<crate::db::Col<R, U>>,
    ) -> Result<crate::db::Cond<R>, Error>
    where
        R: crate::db::Record,
        T: crate::db::DbValue,
        U: crate::db::DbValue,
    {
        let reach = self.reach(permission).ok_or_else(|| self.refuse(permission))?;
        self.fresh_for(permission)?;
        let condition = reach.condition_on(org.sq(), owner.map(crate::db::Col::sq));
        Ok(crate::db::Cond::from_condition(condition))
    }

    /// Checks `permission` on a record (read / edit / delete) or the target node of a create / move command:
    /// missing permission -> `permission_denied`; outside the scope -> `org_out_of_scope`. For a read outside
    /// the scope the service returns 404 instead of this error (the record's existence is not revealed).
    pub async fn require_at<D: ConnectionTrait>(
        &self,
        db: &D,
        permission: &Permission,
        target: OrgTarget,
    ) -> Result<(), Error> {
        match self.check(db, permission, target).await? {
            Decision::Allow => Ok(()),
            Decision::Missing => Err(self.refuse(permission)),
            Decision::OutOfScope => Err(org_out_of_scope(target.org, permission)),
        }
    }

    /// Like [`require_at`](Self::require_at), plus: the target node and all its ancestors must be active
    /// (otherwise `org_frozen`). Call it INSIDE the module's write transaction.
    pub async fn require_write_at<D: ConnectionTrait>(
        &self,
        transaction: &D,
        permission: &Permission,
        target: OrgTarget,
    ) -> Result<(), Error> {
        self.require_at(transaction, permission, target).await?;
        OrgTree::ensure_writable(transaction, self.tenant_id(), target.org).await
    }

    /// Checks that the caller holds `permission` at the tenant's root node: a grant held through the
    /// membership of the root (or every permission), and no fence below the root. For data of the whole
    /// tenant that has no node (tenant settings): a permission that is not `scoped` held through a
    /// branch membership counts across the tenant for records, but must not change what the whole
    /// tenant shares. Missing -> `permission_denied`; held only below the root -> `org_out_of_scope`.
    pub fn require_at_root(&self, permission: &Permission) -> Result<(), Error> {
        self.require(permission)?;
        let root = root_of(self.tenant_id());
        // Nhân sự nền tảng vào khách bằng `kernel.tenants.operate` giữ grant ở nút gốc của khách nền
        // tảng (`operating`); uỷ quyền và hỗ trợ dựng grant ở nút của khách đích — không trùng gốc nhà.
        let home_root = self.operator_from().map(root_of);
        // `Principal::new` (độ sâu `tenant`, D-303 mục 3) đặt grant ở gốc của khách mặc định rồi `in_tenant` đổi khách mà
        // không dời grant: grant đó nghĩa là "cả khách", không phải một nhánh. Gốc khách mặc định không bao giờ là nút
        // nhánh của khách khác (ID nút là ID riêng của từng khách).
        let legacy_root = root_of(crate::tenancy::DEFAULT_TENANT);
        // Grant của permission không `scoped` ở nút nhánh vẫn có độ sâu `tenant` (D-303 mục 3), nên
        // phải xem nút của grant gốc (`grant_map`), không xem `grants()` (đã mở rộng ra cả khách).
        let at_root = match &self.grants {
            Grants::All => true,
            Grants::Only(map) => map.get(permission.name).is_some_and(|grants| {
                grants.iter().any(|grant| {
                    grant.org == root || grant.org == legacy_root || Some(grant.org) == home_root
                })
            }),
        };
        if at_root && self.fence().is_none() {
            Ok(())
        } else {
            Err(org_out_of_scope(root, permission))
        }
    }
}
