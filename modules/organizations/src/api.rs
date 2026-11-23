//! Contract of the `organizations` module for other modules (`autoxide_organizations::api::…`). Get the
//! trait at startup: `context.require::<dyn OrgDirectory>()?`; that module registers AFTER
//! `organizations` in `crates/app/src/modules.rs`.
//!
//! Filtering data by branch does not need this module: use the kernel's `Principal::scope_condition` /
//! `require_at` and `OrgTree`. The trait here covers the meaning of a node (name, kind, status)
//! — e.g. `auth` checks that a node really exists when granting a membership.

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{OrgId, TenantContext};

/// Status of a node. Only an `active` node (with every node above it `active`) can write data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum UnitStatus {
    /// Waiting for someone with `organizations.units.manage` to approve.
    Pending,
    Active,
    Suspended,
    Archived,
}

impl UnitStatus {
    /// Stable names: `pending`, `active`, `suspended`, `archived`.
    pub const fn as_str(self) -> &'static str {
        match self {
            UnitStatus::Pending => "pending",
            UnitStatus::Active => "active",
            UnitStatus::Suspended => "suspended",
            UnitStatus::Archived => "archived",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "pending" => Some(UnitStatus::Pending),
            "active" => Some(UnitStatus::Active),
            "suspended" => Some(UnitStatus::Suspended),
            "archived" => Some(UnitStatus::Archived),
            _ => None,
        }
    }
}

/// A node of the organization tree.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Unit {
    pub id: OrgId,
    /// `None` only for the root node.
    pub parent_id: Option<OrgId>,
    pub unit_type: String,
    pub name: String,
    pub code: Option<String>,
    pub status: UnitStatus,
}

impl Unit {
    /// An active node (for fake `OrgDirectory` implementations in other modules' tests).
    pub fn new(
        id: OrgId,
        parent_id: Option<OrgId>,
        unit_type: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Unit {
            id,
            parent_id,
            unit_type: unit_type.into(),
            name: name.into(),
            code: None,
            status: UnitStatus::Active,
        }
    }
}

/// The effective value of a setting at a node (settings inherit down the tree, step 34 item 4).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct EffectiveSetting {
    pub key: String,
    /// `None`: no node set it and the setting has no default.
    pub value: Option<serde_json::Value>,
    /// The node that set the value; `None`: the default value.
    pub source_unit_id: Option<OrgId>,
    /// Whether the value comes from another (ancestor) node or is the default, not set by the node itself.
    pub inherited: bool,
}

impl EffectiveSetting {
    /// The effective value (for fake `OrgDirectory` implementations in other modules' tests).
    pub fn new(
        key: impl Into<String>,
        value: Option<serde_json::Value>,
        source_unit_id: Option<OrgId>,
        inherited: bool,
    ) -> Self {
        EffectiveSetting { key: key.into(), value, source_unit_id, inherited }
    }
}

/// Opening hours of one weekday, in the node's local time (public profile, step 34).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct OpeningRange {
    /// Weekday per ISO: 1 Monday … 7 Sunday.
    pub day: u8,
    pub opens: jiff::civil::Time,
    pub closes: jiff::civil::Time,
}

impl OpeningRange {
    pub fn new(day: u8, opens: jiff::civil::Time, closes: jiff::civil::Time) -> Self {
        OpeningRange { day, opens, closes }
    }
}

/// Lookup of nodes in the organization tree.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait OrgDirectory: Send + Sync {
    /// Node `id` of the tenant, or `None`.
    async fn unit(&self, tenant: &TenantContext, id: OrgId) -> Result<Option<Unit>, Error>;
    /// The nodes in `ids` that really exist in the tenant (any order; a missing ID is absent). For a page
    /// listing many sellers: one lookup instead of one per node. Default: calls [`Self::unit`] for each ID
    /// (the real implementation reads with one statement).
    async fn units(&self, tenant: &TenantContext, ids: &[OrgId]) -> Result<Vec<Unit>, Error> {
        let mut found = Vec::new();
        for id in ids {
            if let Some(unit) = self.unit(tenant, *id).await? {
                found.push(unit);
            }
        }
        Ok(found)
    }
    /// The tenant's root node (created if missing — tolerates a late "new tenant" event).
    async fn root(&self, tenant: &TenantContext) -> Result<Unit, Error>;
    /// The nodes above `id`, nearest first, including the root; not including the node itself.
    async fn ancestors(&self, tenant: &TenantContext, id: OrgId) -> Result<Vec<Unit>, Error>;
    /// Whether `id` belongs to the branch of `root` (including `root` itself).
    async fn is_within(
        &self,
        tenant: &TenantContext,
        id: OrgId,
        root: OrgId,
    ) -> Result<bool, Error>;
    /// The effective value of setting `key` at node `id` (the nearest node up the chain that sets it, or the
    /// default). A setting nobody declared → `organizations.setting_unknown`.
    async fn setting(
        &self,
        tenant: &TenantContext,
        id: OrgId,
        key: &str,
    ) -> Result<Option<EffectiveSetting>, Error>;

    /// Opening hours in the public profile of node `id`; `None` when the node has not declared any (the
    /// caller uses its own default hours). Default: `None`.
    async fn opening_hours(
        &self,
        _tenant: &TenantContext,
        _id: OrgId,
    ) -> Result<Option<Vec<OpeningRange>>, Error> {
        Ok(None)
    }
}

/// Node groups outside the tree (step 38, D-232): `organizations` publishes them, `reporting` gets them with
/// `context.require::<dyn OrgGroups>()` to filter / aggregate reports. A group grants no permission.
#[async_trait::async_trait]
pub trait OrgGroups: Send + Sync {
    /// Member nodes of group `group`; `None` when the tenant has no such group. NOT filtered by permission —
    /// the caller intersects with the viewer's scope.
    async fn members(
        &self,
        tenant: &TenantContext,
        group: uuid::Uuid,
    ) -> Result<Option<Vec<OrgId>>, Error>;
}
