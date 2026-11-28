//! Narrowing a caller to what a snapshot of earlier permissions also allows — scope included (ROADMAP step 70
//! P16, D-20261128-bound-keeps-scope).
//!
//! [`Principal::only`] keeps the permissions a list names, each with the grants (node, depth) the caller has now.
//! That is enough for an API key (the key carries names only), but not for something that snapshotted a whole
//! caller and runs later with the owner's CURRENT permissions (an automation rule): an owner who held
//! `orders.write` at one branch when the rule was saved and is later given it for the whole tenant would hand the
//! rule the wider reach. [`Principal::bounded_by`] keeps, per permission, only the part both callers reach.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use super::{Grant, Grants, PermissionCatalog, Principal, Scope};
use crate::error::Error;
use crate::tenancy::{OrgId, OrgTree, TenantContext, root_of};

impl Principal {
    /// Keeps only what `limit` also allows: per permission, the part of the tree both reach (a grant of one
    /// side inside a grant of the other keeps the narrower one; grants at unrelated units drop). Every other
    /// property of `self` (user, tenant, fence, credential, banned permissions) stays. A permission that is not
    /// `scoped` in `catalog` takes effect across the whole tenant whatever its grants, so having it on both
    /// sides keeps `self`'s grants; a name `catalog` does not know is treated as scoped.
    ///
    /// Reads the tenant's organization tree only when one side has a `subtree` grant at a unit other than the
    /// other side's.
    pub async fn bounded_by(
        mut self,
        tenant: &TenantContext,
        limit: &Principal,
        catalog: &PermissionCatalog,
    ) -> Result<Self, Error> {
        let Grants::Only(narrow) = &limit.grants else { return Ok(self) };
        let root = root_of(self.tenant);
        let mut tree = Tree { tenant, memo: HashMap::new() };
        let mut kept: BTreeMap<String, Vec<Grant>> = BTreeMap::new();
        for (name, bounds) in narrow.iter() {
            let current = match &self.grants {
                Grants::All => vec![Grant::new(root, Scope::Tenant)],
                Grants::Only(map) => map.get(name).cloned().unwrap_or_default(),
            };
            if current.is_empty() || bounds.is_empty() {
                continue;
            }
            let scoped = catalog
                .iter()
                .find(|permission| permission.name == name)
                .is_none_or(|permission| permission.is_scoped());
            if !scoped {
                kept.insert(name.clone(), current);
                continue;
            }
            let mut both = Vec::new();
            for a in &current {
                for b in bounds {
                    let narrower = if tree.covers(*b, *a).await? {
                        Some(*a)
                    } else if tree.covers(*a, *b).await? {
                        Some(*b)
                    } else {
                        None
                    };
                    if let Some(grant) = narrower
                        && !both.contains(&grant)
                    {
                        both.push(grant);
                    }
                }
            }
            if !both.is_empty() {
                kept.insert(name.clone(), both);
            }
        }
        self.grants = Grants::Only(Arc::new(kept));
        Ok(self)
    }
}

/// The tenant's tree, asked at most once per pair of units.
struct Tree<'a> {
    tenant: &'a TenantContext,
    memo: HashMap<(OrgId, OrgId), bool>,
}

impl Tree<'_> {
    /// Whether everything grant `inner` reaches is also reached by grant `outer`. Regions of the tree are
    /// either nested or apart, so "the narrower of two nested grants" is exactly their intersection.
    async fn covers(&mut self, outer: Grant, inner: Grant) -> Result<bool, Error> {
        Ok(match (outer.scope, inner.scope) {
            (Scope::Tenant, _) => true,
            (_, Scope::Tenant) => false,
            (Scope::Subtree, _) if outer.org == inner.org => true,
            (Scope::Subtree, _) => self.within(inner.org, outer.org).await?,
            (Scope::Unit, Scope::Unit | Scope::Own) | (Scope::Own, Scope::Own) => {
                outer.org == inner.org
            }
            (Scope::Unit | Scope::Own, _) => false,
        })
    }

    async fn within(&mut self, unit: OrgId, branch: OrgId) -> Result<bool, Error> {
        if let Some(known) = self.memo.get(&(unit, branch)) {
            return Ok(*known);
        }
        let within =
            OrgTree::is_within(self.tenant.database().read(), self.tenant.id(), unit, &[branch])
                .await?;
        self.memo.insert((unit, branch), within);
        Ok(within)
    }
}
