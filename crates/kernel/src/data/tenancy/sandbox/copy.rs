//! Copying configuration into a sandbox and pushing what was tried to the real tenant (step 38, D-232,
//! D-20261119-step38-sandbox-copy).
//!
//! - A module declares [`SandboxCopy`] through `Module::sandbox_copy`: the `kernel.sandbox_copy` job calls them
//!   in registration order, after seeding the new tenant's data for the sandbox and before staff can enter. It
//!   copies CONFIGURATION only; never users, buyers, orders, accounting documents, notifications, private
//!   files, or secrets. A copy gets a new ID; it is linked to the original by [`link_copy`] (table
//!   `kernel_sandbox_links`), so a rerun creates no duplicate and references between records follow the
//!   original ([`copied`]).
//! - A module declares [`Promotable`] through `Module::promotable`: `POST /kernel/sandboxes/{id}/promote`
//!   calls [`PromoteApply::promote`] in the real tenant's context, with the caller's own permissions. The
//!   real tenant always receives a DRAFT (never auto-published or enabled), and never overwrites the running
//!   version.

use crate::time::Timestamp;
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, Set};
use std::sync::{Arc, OnceLock};
use uuid::Uuid;

use crate::access::Principal;
use crate::error::Error;
use crate::tenancy::{ScopeExt, TenantContext, TenantScoped};

pub(crate) mod entity {
    //! The `kernel_sandbox_links` table (migration `m0038_create_sandbox_tables`).
    use crate::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "kernel_sandbox_links")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        /// The sandbox that owns the row.
        pub tenant_id: Uuid,
        /// Record kind `<module>.<kind>`.
        pub kind: String,
        /// The record in the sandbox.
        pub record_id: Uuid,
        /// The original record at the real tenant it was copied from; `None`: created new in the sandbox.
        pub source_id: Option<Uuid>,
        /// The record at the real tenant that the most recent push created / edited.
        pub promoted_id: Option<Uuid>,
        /// Content fingerprint of the most recent push (pushing the same content again → `unchanged`).
        pub promoted_fingerprint: Option<String>,
        pub created_at: Timestamp,
        pub updated_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

impl TenantScoped for entity::Entity {
    fn tenant_column() -> Self::Column {
        entity::Column::TenantId
    }
}

/// Options when creating a sandbox.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SandboxOptions {
    /// Maximum number of products to copy (0 when not given).
    pub products: u32,
}

/// What one copy receives.
#[non_exhaustive]
pub struct CopyContext<'a> {
    /// The real tenant (read).
    pub source: &'a TenantContext,
    /// The sandbox (write, through the module's service).
    pub sandbox: &'a TenantContext,
    pub options: SandboxOptions,
    /// The actor of the copy job: every permission inside the sandbox (the sandbox belongs to the tenant
    /// itself, and the job was created by the tenant owner). Never used at the real tenant.
    pub principal: &'a Principal,
}

/// A module copies its configuration into the sandbox.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait SandboxCopy: Send + Sync {
    /// Copies configuration from `context.source` to `context.sandbox`; returns the number of records copied
    /// this time. Must be re-runnable: a record already copied ([`copied`]) is skipped. Never copies personal
    /// data, transactions or secrets.
    async fn copy(&self, context: &CopyContext<'_>) -> Result<u64, Error>;
}

/// One item to push to the real tenant.
#[non_exhaustive]
pub struct PromoteRequest<'a> {
    pub sandbox: &'a TenantContext,
    /// The real tenant (the sandbox's source tenant).
    pub target: &'a TenantContext,
    /// The caller, at the real tenant: the module checks its own permission as REST does.
    pub principal: &'a Principal,
    /// The record in the sandbox.
    pub id: Uuid,
    /// The record at the real tenant that should receive the change: the one the previous push created, or
    /// failing that the original the sandbox record was copied from. `None`: create a new draft. The module
    /// checks that this record still exists.
    pub existing: Option<Uuid>,
}

/// Result of pushing one item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Promoted {
    /// Created a new draft at the real tenant.
    CreatedDraft(Uuid),
    /// Edited the draft / pending revision of the record at the real tenant (the running version stays as is).
    UpdatedDraft(Uuid),
    /// Nothing differs from what was pushed.
    Unchanged(Uuid),
    /// Name / path / code conflict at the real tenant: this item is not pushed, other items still run.
    Conflict { field: String, message: String },
}

/// A module pushes one record kind from the sandbox to the real tenant.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait PromoteApply: Send + Sync {
    /// Writes record `request.id` of the sandbox to the real tenant through the module's service, ALWAYS as a
    /// draft / disabled. The `fingerprint` of the pushed content is kept by the kernel: the module returns
    /// `Unchanged` when the content equals `previous` (the same hash function the module uses).
    async fn promote(
        &self,
        request: &PromoteRequest<'_>,
        previous: Option<&str>,
    ) -> Result<(Promoted, Option<String>), Error>;
}

/// One record kind that can be pushed to the real tenant.
#[derive(Clone)]
#[non_exhaustive]
pub struct Promotable {
    /// `<module>.<kind>`, vd `content.page`.
    pub kind: &'static str,
    /// English description for API callers.
    pub description: &'static str,
    pub apply: Arc<dyn PromoteApply>,
}

impl Promotable {
    pub fn new(
        kind: &'static str,
        description: &'static str,
        apply: Arc<dyn PromoteApply>,
    ) -> Self {
        Promotable { kind, description, apply }
    }
}

/// Registry of copy / push of every module, filled once after every module starts. Cheap to clone.
#[derive(Clone, Default)]
pub(crate) struct SandboxCatalog(Arc<OnceLock<CatalogInner>>);

struct CatalogInner {
    copies: Vec<(&'static str, Arc<dyn SandboxCopy>)>,
    promotables: Vec<(&'static str, Promotable)>,
}

impl SandboxCatalog {
    pub(crate) fn fill(
        &self,
        copies: Vec<(&'static str, Arc<dyn SandboxCopy>)>,
        promotables: Vec<(&'static str, Promotable)>,
    ) {
        let _ = self.0.set(CatalogInner { copies, promotables });
    }

    pub(crate) fn copies(&self) -> Vec<(&'static str, Arc<dyn SandboxCopy>)> {
        self.0.get().map(|inner| inner.copies.clone()).unwrap_or_default()
    }

    pub(crate) fn promotables(&self) -> Vec<(&'static str, Promotable)> {
        self.0.get().map(|inner| inner.promotables.clone()).unwrap_or_default()
    }
}

fn now() -> Timestamp {
    Timestamp::now()
}

/// Records that sandbox record `record_id` is a copy of `source_id` (kind `kind`). Calling again adds no
/// second row.
pub async fn link_copy(
    sandbox: &TenantContext,
    kind: &str,
    record_id: Uuid,
    source_id: Uuid,
) -> Result<(), Error> {
    let at = now();
    let row = entity::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(sandbox.id().as_uuid()),
        kind: Set(kind.to_owned()),
        record_id: Set(record_id),
        source_id: Set(Some(source_id)),
        promoted_id: Set(None),
        promoted_fingerprint: Set(None),
        created_at: Set(at),
        updated_at: Set(at),
    };
    entity::Entity::insert(row)
        .on_conflict(
            OnConflict::columns([
                entity::Column::TenantId,
                entity::Column::Kind,
                entity::Column::RecordId,
            ])
            .do_nothing()
            .to_owned(),
        )
        .exec_without_returning(sandbox.database().write())
        .await?;
    Ok(())
}

/// The copy in the sandbox of the original record `source_id` (kind `kind`), if already copied.
pub async fn copied(
    sandbox: &TenantContext,
    kind: &str,
    source_id: Uuid,
) -> Result<Option<Uuid>, Error> {
    Ok(entity::Entity::find()
        .filter(entity::Column::Kind.eq(kind))
        .filter(entity::Column::SourceId.eq(source_id))
        .scoped(sandbox)
        .one(sandbox.database().read())
        .await?
        .map(|row| row.record_id))
}

/// The original record at the real tenant of record `record_id` in the sandbox, or the one the previous push created.
pub async fn promoted_target(
    sandbox: &TenantContext,
    kind: &str,
    record_id: Uuid,
) -> Result<Option<Uuid>, Error> {
    Ok(link_of(sandbox, kind, record_id).await?.and_then(|row| row.promoted_id.or(row.source_id)))
}

pub(crate) async fn link_of(
    sandbox: &TenantContext,
    kind: &str,
    record_id: Uuid,
) -> Result<Option<entity::Model>, Error> {
    Ok(entity::Entity::find()
        .filter(entity::Column::Kind.eq(kind))
        .filter(entity::Column::RecordId.eq(record_id))
        .scoped(sandbox)
        .one(sandbox.database().read())
        .await?)
}

/// Records the result of pushing `record_id` (the record at the real tenant, the content fingerprint).
pub(crate) async fn record_promotion(
    sandbox: &TenantContext,
    kind: &str,
    record_id: Uuid,
    promoted_id: Uuid,
    fingerprint: Option<String>,
) -> Result<(), Error> {
    let at = now();
    match link_of(sandbox, kind, record_id).await? {
        Some(row) => {
            entity::Entity::update_many()
                .col_expr(entity::Column::PromotedId, Expr::value(Some(promoted_id)))
                .col_expr(entity::Column::PromotedFingerprint, Expr::value(fingerprint))
                .col_expr(entity::Column::UpdatedAt, Expr::value(at))
                .filter(entity::Column::Id.eq(row.id))
                .exec(sandbox.database().write())
                .await?;
        }
        None => {
            entity::Entity::insert(entity::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(sandbox.id().as_uuid()),
                kind: Set(kind.to_owned()),
                record_id: Set(record_id),
                source_id: Set(None),
                promoted_id: Set(Some(promoted_id)),
                promoted_fingerprint: Set(fingerprint),
                created_at: Set(at),
                updated_at: Set(at),
            })
            .exec(sandbox.database().write())
            .await?;
        }
    }
    Ok(())
}
