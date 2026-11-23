//! The `kernel_relocations` table (control plane): each split / merge run, or tenant request, is one row that
//! goes through the states of [`super::run`].

use crate::time::Timestamp;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{RelocationKind, RelocationPlan, TenantRelocation};
use crate::error::Error;
use crate::migration::prelude::*;
use crate::tenancy::TenantTableRows;

pub(crate) const REQUESTED: &str = "requested";
pub(crate) const PLANNED: &str = "planned";
pub(crate) const FROZEN: &str = "frozen";
pub(crate) const COPIED: &str = "copied";
pub(crate) const VERIFIED: &str = "verified";
pub(crate) const SWITCHED: &str = "switched";
pub(crate) const CLEANED: &str = "cleaned";
pub(crate) const DONE: &str = "done";
pub(crate) const FAILED: &str = "failed";
/// States of a run in progress: the source tenant (and the receiving tenant on a merge) is not served.
pub(crate) const UNFINISHED: [&str; 6] = [PLANNED, FROZEN, COPIED, VERIFIED, SWITCHED, CLEANED];

pub(crate) mod entity {
    //! The `kernel_relocations` table (control plane).
    use crate::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "kernel_relocations")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        /// `split` / `merge`.
        pub kind: String,
        pub state: String,
        pub source_tenant_id: Uuid,
        /// Split: ID of the new tenant (= branch root node). Merge: the receiving tenant.
        pub target_tenant_id: Option<Uuid>,
        /// Split: the branch root node. Merge: the receiving node of the receiving tenant.
        pub org_id: Option<Uuid>,
        /// Split: placement of the new tenant.
        pub placement_id: Option<Uuid>,
        /// JSON [`super::Options`].
        pub options: String,
        /// Plan JSON at start (or when the tenant sent the request).
        pub plan: Option<String>,
        /// JSON `[{"table": …, "rows": …}]`: rows written at the target for each table.
        pub tables: String,
        pub error: Option<String>,
        pub runner: Option<String>,
        pub runner_until: Option<Timestamp>,
        /// The tenant's person who sent the request (`requested`).
        pub requested_by: Option<Uuid>,
        pub created_by: Option<Uuid>,
        pub created_at: Timestamp,
        pub updated_at: Timestamp,
        pub finished_at: Option<Timestamp>,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) use entity::{Column, Entity, Model};

/// Options of a run, stored in the `options` column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Options {
    /// Split: slug and name of the new tenant.
    #[serde(default)]
    pub(crate) slug: Option<String>,
    #[serde(default)]
    pub(crate) name: Option<String>,
    /// Split: the new tenant is a child of the source tenant.
    #[serde(default)]
    pub(crate) keep_parent: bool,
    /// Reason (required: transferring data between two tenants; legal basis, D-20261118-step38-relocation).
    #[serde(default)]
    pub(crate) reason: String,
    /// Request: the placement name the tenant wants (platform staff decide).
    #[serde(default)]
    pub(crate) placement: Option<String>,
    /// Merge: the receiving tenant (ID or slug); a request has no `target_tenant_id` yet.
    #[serde(default)]
    pub(crate) into: Option<String>,
    /// Root tenant of the tree before and after (for `kernel.relocation_finished`, moving the reporting store).
    #[serde(default)]
    pub(crate) from_root: Option<Uuid>,
    #[serde(default)]
    pub(crate) to_root: Option<Uuid>,
    /// When the tenant began maintenance (waiting for empty queues counts from here).
    #[serde(default)]
    pub(crate) frozen_at: Option<Timestamp>,
}

impl Options {
    pub(crate) fn parse(raw: &str) -> Result<Self, Error> {
        serde_json::from_str(raw).map_err(Error::internal)
    }

    pub(crate) fn json(&self) -> Result<String, Error> {
        serde_json::to_string(self).map_err(Error::internal)
    }
}

/// A saved run, for callers.
pub(crate) fn describe(row: Model, placement: Option<String>) -> Result<TenantRelocation, Error> {
    let options = Options::parse(&row.options)?;
    let plan: Option<RelocationPlan> = match &row.plan {
        Some(raw) => Some(serde_json::from_str(raw).map_err(Error::internal)?),
        None => None,
    };
    let tables: Vec<TenantTableRows> =
        serde_json::from_str(&row.tables).map_err(Error::internal)?;
    Ok(TenantRelocation {
        id: row.id,
        kind: RelocationKind::parse(&row.kind).ok_or_else(|| {
            Error::internal(format!("relocation {} has kind `{}`", row.id, row.kind))
        })?,
        state: row.state,
        source_tenant_id: row.source_tenant_id,
        target_tenant_id: row.target_tenant_id,
        org_id: row.org_id,
        placement: placement.or(options.placement),
        reason: options.reason,
        plan,
        tables,
        error: row.error,
        created_at: row.created_at,
        updated_at: row.updated_at,
        finished_at: row.finished_at,
    })
}

/// The `kernel_relocations` table (control plane; only the control database holds data).
pub(crate) struct CreateRelocations;

impl MigrationName for CreateRelocations {
    fn name(&self) -> &str {
        "m0035_create_relocations"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateRelocations {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("kernel_relocations")
                    .col(schema::pk_uuid("id"))
                    .col(schema::string_len("kind", 16))
                    .col(schema::string_len("state", 16))
                    .col(schema::uuid("source_tenant_id"))
                    .col(schema::uuid_null("target_tenant_id"))
                    .col(schema::uuid_null("org_id"))
                    .col(schema::uuid_null("placement_id"))
                    .col(schema::text("options"))
                    .col(schema::text_null("plan"))
                    .col(schema::text("tables"))
                    .col(schema::text_null("error"))
                    .col(schema::string_len_null("runner", 64))
                    .col(schema::timestamp_with_time_zone_null("runner_until"))
                    .col(schema::uuid_null("requested_by"))
                    .col(schema::uuid_null("created_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .col(schema::timestamp_with_time_zone_null("finished_at"))
                    .to_owned(),
            )
            .await?;
        for (name, columns) in [
            ("kernel_relocations_source_idx", ["source_tenant_id", "state"]),
            ("kernel_relocations_target_idx", ["target_tenant_id", "state"]),
            ("kernel_relocations_state_idx", ["state", "created_at"]),
        ] {
            let mut index = Index::create();
            index.name(name).table("kernel_relocations");
            for column in columns {
                index.col(column);
            }
            manager.create_index(index.to_owned()).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("kernel_relocations").to_owned()).await
    }
}

/// New ID for a run.
pub(crate) fn new_id() -> Uuid {
    Uuid::now_v7()
}
