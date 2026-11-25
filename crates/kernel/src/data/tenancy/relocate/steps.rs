//! The steps of one split / merge run. Each step is re-runnable; there is no transaction across the two
//! databases: copying is ONE transaction at the target, switching is one transaction at the control database,
//! and cleaning is one transaction at the source and deletes only rows that already exist at the target.

use std::collections::HashSet;

use sea_orm::sea_query::{Expr, Value};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use serde::Serialize;
use uuid::Uuid;

use super::plan::pending;
use super::policy::{Mode, Resolved, Treatment};
use super::prepare::Prepared;
use super::run::{TenantRelocations, now};
use super::store::{self, Column, Model, Options};
use super::{RelocationContext, RelocationKind, errors};
use crate::error::Error;
use crate::event::{Actor, Event};
use crate::secret::{Sealer, is_sealed};
use crate::tenancy::admin::tenant;
use crate::tenancy::{PlacementId, TenantContext, TenantId, TenantStatus, TenantTableRows};
use crate::transfer::{CopiedTable, Row, TenantTables, TransferError, primary_key, tenant_is};

fn transfer(error: TransferError) -> Error {
    Error::internal(error.to_string())
}

/// Content of the `kernel.relocation_*` events.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct RelocationEvent {
    relocation_id: Uuid,
    kind: RelocationKind,
    source_tenant_id: Uuid,
    target_tenant_id: Option<Uuid>,
    org_id: Option<Uuid>,
    /// Root tenant of the tree that holds the data before and after (moving the combined reporting store).
    from_root: Option<Uuid>,
    to_root: Option<Uuid>,
}

impl RelocationEvent {
    fn of(row: &Model, kind: RelocationKind, options: &Options) -> Self {
        RelocationEvent {
            relocation_id: row.id,
            kind,
            source_tenant_id: row.source_tenant_id,
            target_tenant_id: row.target_tenant_id,
            org_id: row.org_id,
            from_root: options.from_root,
            to_root: options.to_root,
        }
    }
}

/// `kernel.relocation_started`.
#[derive(Debug, Serialize)]
#[serde(transparent)]
struct Started(RelocationEvent);

impl Event for Started {
    const NAME: &'static str = "kernel.relocation_started";
}

/// `kernel.relocation_finished`.
#[derive(Debug, Serialize)]
#[serde(transparent)]
struct Finished(RelocationEvent);

impl Event for Finished {
    const NAME: &'static str = "kernel.relocation_finished";
}

/// `kernel.relocation_failed`.
#[derive(Debug, Serialize)]
#[serde(transparent)]
struct Failed(RelocationEvent);

impl Event for Failed {
    const NAME: &'static str = "kernel.relocation_failed";
}

impl TenantRelocations {
    /// Sets `status` for tenants `ids` when the tenant is in one of `from`.
    async fn set_status<C: sea_orm::ConnectionTrait>(
        &self,
        connection: &C,
        ids: &[Uuid],
        from: &[TenantStatus],
        status: TenantStatus,
    ) -> Result<u64, Error> {
        let updated = tenant::Entity::update_many()
            .col_expr(tenant::Column::Status, Expr::value(status.as_str()))
            .col_expr(tenant::Column::UpdatedAt, Expr::value(now()))
            .filter(tenant::Column::Id.is_in(ids.iter().copied()))
            .filter(tenant::Column::Status.is_in(from.iter().map(|status| status.as_str())))
            .exec(connection)
            .await?;
        for id in ids {
            self.registry.invalidate(TenantId::from_uuid(*id));
        }
        Ok(updated.rows_affected)
    }

    /// Tenants locked during the run: the source tenant, and the receiving tenant on a merge.
    fn frozen_tenants(row: &Model, kind: RelocationKind) -> Vec<Uuid> {
        let mut ids = vec![row.source_tenant_id];
        if kind == RelocationKind::Merge {
            ids.extend(row.target_tenant_id);
        }
        ids
    }

    /// Records the event `event` for tenant `tenant` in its own database (outside other transactions).
    async fn record<E: Event>(
        &self,
        tenant: Uuid,
        actor: Actor,
        relocation: Uuid,
        event: &E,
    ) -> Result<(), Error> {
        let context = self.registry.tenant(TenantId::from_uuid(tenant)).await?;
        let subject = Some(format!("kernel.relocation:{relocation}"));
        self.outbox.record(context.database().write(), &context, actor, subject, event).await
    }

    /// `planned` → `frozen`: the tenant becomes `maintenance` (requests get 503, workers keep running), waits
    /// for the tenant's due outbox / jobs to finish, then `moving` (workers and the dispatcher stop the
    /// tenant's work), then waits for other servers to notice and for work in progress to finish.
    pub(super) async fn freeze(&self, row: &Model, runner: &str) -> Result<(), Error> {
        let spec = Self::spec_of(row)?;
        let mut options = spec.options.clone();
        let ids = Self::frozen_tenants(row, spec.kind);
        let actor = row.created_by.map_or_else(Actor::system, Actor::user);
        if options.frozen_at.is_none() {
            let transaction = self.control().begin_write().await?;
            let changed = self
                .set_status(&transaction, &ids, &[TenantStatus::Active], TenantStatus::Maintenance)
                .await?;
            if changed != ids.len() as u64 {
                return Err(errors::not_active(&row.source_tenant_id.to_string(), "not active"));
            }
            options.frozen_at = Some(now());
            Entity::update_options(&transaction, row.id, &options).await?;
            transaction.commit().await?;
        }
        let prepared = self.prepare(&spec, Some(row)).await?;
        let (source, _) = databases(&prepared)?;
        let (tables, _) = self.resolve(&prepared, &source, &source).await?;
        let started = options.frozen_at.unwrap_or_else(now);
        loop {
            let mut waiting = 0;
            for (table, kind) in tables.queues() {
                waiting += tables
                    .count_matching(
                        &source,
                        table,
                        pending(kind, TenantId::from_uuid(row.source_tenant_id)),
                    )
                    .await
                    .map_err(transfer)?;
            }
            if waiting == 0 {
                break;
            }
            let waited = std::time::Duration::try_from(now() - started).unwrap_or_default();
            if waited > self.drain {
                return self.fail(row, runner, &spec.kind, errors::drain_timeout(waiting)).await;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        let transaction = self.control().begin_write().await?;
        self.set_status(
            &transaction,
            &ids,
            &[TenantStatus::Maintenance, TenantStatus::Moving],
            TenantStatus::Moving,
        )
        .await?;
        transaction.commit().await?;
        // Ghi SAU khi hàng đợi đã rỗng (event này không phải chờ chính nó): giao khi khách mở lại.
        let event = Started(RelocationEvent::of(row, spec.kind, &options));
        self.record(row.source_tenant_id, actor, row.id, &event).await?;
        tokio::time::sleep(self.settle()).await;
        crate::tenancy::moves::wait_claims(
            &source,
            &tables,
            TenantId::from_uuid(row.source_tenant_id),
        )
        .await?;
        self.set_state(self.control().write(), row, runner, store::FROZEN, Ok).await
    }

    /// Stops the run before anything changes: the tenant is served again, the run is `failed`.
    async fn fail(
        &self,
        row: &Model,
        runner: &str,
        kind: &RelocationKind,
        error: Error,
    ) -> Result<(), Error> {
        let ids = Self::frozen_tenants(row, *kind);
        let message = crate::tenancy::moves::describe(&error);
        let transaction = self.control().begin_write().await?;
        self.set_status(
            &transaction,
            &ids,
            &[TenantStatus::Maintenance, TenantStatus::Moving],
            TenantStatus::Active,
        )
        .await?;
        self.set_state(&transaction, row, runner, store::FAILED, |query| {
            Ok(query.col_expr(Column::Error, Expr::value(message.clone())))
        })
        .await?;
        transaction.commit().await?;
        let options = Options::parse(&row.options)?;
        let actor = row.created_by.map_or_else(Actor::system, Actor::user);
        let event = Failed(RelocationEvent::of(row, *kind, &options));
        self.record(row.source_tenant_id, actor, row.id, &event).await?;
        tracing::warn!(relocation_id = %row.id, error = %message, "tenant relocation failed; the tenants work again");
        Ok(())
    }

    /// Re-checks, tables and context of a saved run.
    async fn reopen(
        &self,
        row: &Model,
    ) -> Result<(Prepared, TenantTables, Vec<Resolved>, RelocationContext), Error> {
        let spec = Self::spec_of(row)?;
        let prepared = self.prepare(&spec, Some(row)).await?;
        let (source, target) = databases(&prepared)?;
        let (tables, resolved) = self.resolve(&prepared, &source, &target).await?;
        let context = self.context(row.id, &prepared, &source, &target, &tables, &resolved).await?;
        Ok((prepared, tables, resolved, context))
    }

    /// `frozen` → `copied`: (split) creates the new tenant as `provisioning`; copies in ONE transaction at
    /// the target every table according to policy, keeping primary keys and changing `tenant_id`; then the
    /// `Custom` tables.
    pub(super) async fn copy(&self, row: &Model, runner: &str) -> Result<(), Error> {
        let (mut prepared, tables, resolved, context) = self.reopen(row).await?;
        if !prepared.blockers.is_empty() {
            return Err(prepared.blockers.swap_remove(0));
        }
        let options = Options::parse(&row.options)?;
        if prepared.kind == RelocationKind::Split {
            self.create_target(&prepared, &options).await?;
        }
        // File trước dòng: chép file hỏng thì đích chưa có gì đổi; chạy lại chép lại.
        let files = self
            .copy_blobs(
                &context,
                prepared.source.placement_id,
                prepared_target_placement(&prepared),
            )
            .await?;
        let sealer = self.key.sealer().await?;
        let policies = self.setup.policies()?;
        let merge = prepared.kind == RelocationKind::Merge;
        let target_tenant = context.target_tenant();
        let transaction = context.target().begin_write().await?;
        if !merge {
            // Lần chép trước bị ngắt sau khi commit: đích có dòng của khách mới — chép lại từ đầu.
            tables.delete(&transaction, target_tenant, |_| true).await.map_err(transfer)?;
        }
        let mut copied = Vec::new();
        for table in &resolved {
            let Treatment::Select { condition, .. } = &table.treatment else { continue };
            let sealed = policies.sealed(&table.table);
            let rewrite = |row: &mut Row| rewrite(row, &context, table, &sealed, &sealer);
            let rows = tables
                .copy_where(
                    context.source(),
                    &transaction,
                    &table.table,
                    condition.clone(),
                    &rewrite,
                    merge,
                )
                .await
                .map_err(transfer)?;
            copied.push(CopiedTable::new(table.table.clone(), rows));
        }
        for table in &resolved {
            if let Treatment::Custom(handler) = &table.treatment {
                let rows = handler.copy(&context, &transaction).await?;
                copied.push(CopiedTable::new(table.table.clone(), rows));
            }
        }
        transaction.commit().await?;
        if prepared.kind == RelocationKind::Split {
            let tenant = TenantContext::new(
                target_tenant,
                options.slug.clone().unwrap_or_default(),
                options.name.clone().unwrap_or_default(),
                TenantStatus::Provisioning,
                PlacementId::from_uuid(prepared.placement.id),
                context.target().clone(),
            );
            for (module, seed) in self.seeds.all() {
                seed.seed(&tenant).await.map_err(|error| {
                    Error::internal(format!(
                        "module `{module}` cannot prepare the new tenant: {error}"
                    ))
                })?;
            }
        }
        let rows: Vec<TenantTableRows> = copied.into_iter().map(TenantTableRows::from).collect();
        let json = serde_json::to_string(&rows).map_err(Error::internal)?;
        tracing::info!(relocation_id = %row.id, rows = rows.iter().map(|table| table.rows).sum::<u64>(), files, "tenant rows copied");
        self.set_state(self.control().write(), row, runner, store::COPIED, |query| {
            Ok(query.col_expr(Column::Tables, Expr::value(json)))
        })
        .await
    }

    /// The new tenant on a split (`provisioning`), if not yet present.
    async fn create_target(&self, prepared: &Prepared, options: &Options) -> Result<(), Error> {
        if tenant::Entity::find_by_id(prepared.target_id)
            .one(self.control().write())
            .await?
            .is_some()
        {
            return Ok(());
        }
        let at = now();
        tenant::ActiveModel {
            id: Set(prepared.target_id),
            slug: Set(options.slug.clone().unwrap_or_default()),
            name: Set(options.name.clone().unwrap_or_default()),
            status: Set(TenantStatus::Provisioning.as_str().to_owned()),
            parent_id: Set(options.keep_parent.then_some(prepared.source.id)),
            placement_id: Set(prepared.placement.id),
            created_at: Set(at),
            updated_at: Set(at),
            read_only: Set(false),
            data_residency: Set(prepared.source.data_residency.clone()),
        }
        .insert(self.control().write())
        .await?;
        Ok(())
    }

    /// `copied` → `verified`: every row that goes / is copied (by primary key) already exists at the target.
    pub(super) async fn verify(&self, row: &Model, runner: &str) -> Result<(), Error> {
        let (_, tables, resolved, context) = self.reopen(row).await?;
        for table in &resolved {
            let Treatment::Select { condition, .. } = &table.treatment else { continue };
            let pk: Vec<String> = primary_key(context.source().write(), &table.table)
                .await
                .map_err(transfer)?
                .into_iter()
                .filter(|column| column != "tenant_id")
                .collect();
            let wanted = tables
                .values(context.source().write(), &table.table, &pk, condition.clone())
                .await
                .map_err(transfer)?;
            if wanted.is_empty() {
                continue;
            }
            let found: HashSet<Vec<Option<String>>> = tables
                .values(
                    context.target().write(),
                    &table.table,
                    &pk,
                    tenant_is(context.target_tenant()),
                )
                .await
                .map_err(transfer)?
                .into_iter()
                .collect();
            let present = wanted.iter().filter(|key| found.contains(*key)).count() as u64;
            if present != wanted.len() as u64 {
                // Nguồn đổi sau khi chép (không được xảy ra khi khách `moving`): chép lại.
                let _ =
                    self.set_state(self.control().write(), row, runner, store::FROZEN, Ok).await;
                return Err(errors::count_mismatch(&table.table, wanted.len() as u64, present));
            }
        }
        self.set_state(self.control().write(), row, runner, store::VERIFIED, Ok).await
    }

    /// The root tenant of the tree that contains tenant `tenant`.
    async fn root(&self, tenant: Uuid) -> Result<Uuid, Error> {
        let above = self.registry.tenant_ancestors(TenantId::from_uuid(tenant)).await?;
        Ok(above.last().map_or(tenant, TenantId::as_uuid))
    }

    /// `verified` → `switched`: one transaction at the control database: the new tenant becomes active / the
    /// merged tenant becomes `merged`, domains bound to a node in the branch and child tenants follow; the
    /// source tenant reopens.
    pub(super) async fn switch(&self, row: &Model, runner: &str) -> Result<(), Error> {
        let (prepared, _, _, context) = self.reopen(row).await?;
        let mut options = Options::parse(&row.options)?;
        let (source, target) = (prepared.source.id, prepared.target_id);
        let transaction = self.control().begin_write().await?;
        // Việc đồng bộ của module (vd chép rào quyền) TRƯỚC khi khách mới hoạt động, trong cùng
        // transaction: lỗi thì không gì đổi, khách mới không mở.
        let switch = super::RelocationSwitch {
            kind: prepared.kind,
            source: TenantId::from_uuid(source),
            target: TenantId::from_uuid(target),
        };
        for hook in self.setup.policies()?.switch_hooks() {
            hook.switch(&transaction, &switch).await?;
        }
        let domains: Vec<String>;
        match prepared.kind {
            RelocationKind::Split => {
                let from_root = self.root(source).await?;
                options.from_root = Some(from_root);
                options.to_root = Some(if options.keep_parent { from_root } else { target });
                let branch =
                    super::plan::branch(context.target(), context.target_tenant(), prepared.org)
                        .await?;
                domains = self.move_domains(&transaction, source, target, Some(&branch)).await?;
                self.set_status(
                    &transaction,
                    &[target],
                    &[TenantStatus::Provisioning],
                    TenantStatus::Active,
                )
                .await?;
                self.set_status(
                    &transaction,
                    &[source],
                    &[TenantStatus::Moving],
                    TenantStatus::Active,
                )
                .await?;
            }
            RelocationKind::Merge => {
                options.from_root = Some(self.root(source).await?);
                options.to_root = Some(self.root(target).await?);
                domains = self.move_domains(&transaction, source, target, None).await?;
                tenant::Entity::update_many()
                    .col_expr(tenant::Column::ParentId, Expr::value(target))
                    .col_expr(tenant::Column::UpdatedAt, Expr::value(now()))
                    .filter(tenant::Column::ParentId.eq(source))
                    .exec(&transaction)
                    .await?;
                self.set_status(
                    &transaction,
                    &[source],
                    &[TenantStatus::Moving],
                    TenantStatus::Merged,
                )
                .await?;
                self.set_status(
                    &transaction,
                    &[target],
                    &[TenantStatus::Moving],
                    TenantStatus::Active,
                )
                .await?;
            }
        }
        let json = options.json()?;
        self.set_state(&transaction, row, runner, store::SWITCHED, |query| {
            Ok(query.col_expr(Column::Options, Expr::value(json)))
        })
        .await?;
        transaction.commit().await?;
        for domain in &domains {
            self.registry.invalidate_domain(domain);
        }
        for id in [source, target] {
            self.registry.invalidate(TenantId::from_uuid(id));
        }
        tracing::info!(relocation_id = %row.id, "tenant relocation switched");
        Ok(())
    }

    /// Domains of tenant `from` (only the domains bound to a node in `branch`, if given) to tenant `to`.
    async fn move_domains<C: sea_orm::ConnectionTrait>(
        &self,
        connection: &C,
        from: Uuid,
        to: Uuid,
        branch: Option<&[Uuid]>,
    ) -> Result<Vec<String>, Error> {
        use crate::tenancy::domains::entity as domain;
        let mut select = domain::Entity::find().filter(domain::Column::TenantId.eq(from));
        if let Some(branch) = branch {
            select = select.filter(domain::Column::OrgId.is_in(branch.iter().copied()));
        }
        let found = select.all(connection).await?;
        // Khách nhận có thể đã có dòng cho cùng tên miền (mỗi khách tối đa một dòng một tên miền, P-4): giữ dòng
        // "mạnh" hơn (verified > pending > failed / revoked), xoá dòng kia — dòng thừa của cùng một khách không mang
        // nghĩa gì, còn để lại thì UPDATE vi phạm unique `(domain, tenant_id)` và lượt gộp lặp lỗi mãi.
        let names: Vec<String> = found.iter().map(|row| row.domain.clone()).collect();
        let mut existing = std::collections::HashMap::new();
        for chunk in names.chunks(500) {
            let rows = domain::Entity::find()
                .filter(domain::Column::TenantId.eq(to))
                .filter(domain::Column::Domain.is_in(chunk.iter().cloned()))
                .all(connection)
                .await?;
            existing.extend(rows.into_iter().map(|row| (row.domain.clone(), row)));
        }
        let mut ids = Vec::new();
        let mut dropped = Vec::new();
        for row in &found {
            match existing.get(&row.domain) {
                Some(kept) if claim_rank(&kept.status) >= claim_rank(&row.status) => {
                    dropped.push((row.id, row.status.clone()));
                }
                Some(kept) => {
                    dropped.push((kept.id, kept.status.clone()));
                    ids.push(row.id);
                }
                None => ids.push(row.id),
            }
        }
        // Xoá kèm đúng trạng thái đã đọc: trên Postgres (read committed) job `kernel.verify_domains` hay một lượt
        // xác minh có thể vừa đổi dòng `pending` / `failed` thành `verified` giữa lúc đọc và lúc xoá — không được xoá
        // mất dòng đó. Lệch thì báo lỗi; transaction huỷ và lượt dời chạy lại từ bước này với dữ liệu mới.
        for (id, status) in dropped {
            let deleted = domain::Entity::delete_many()
                .filter(domain::Column::Id.eq(id))
                .filter(domain::Column::Status.eq(status.as_str()))
                .exec(connection)
                .await?;
            if deleted.rows_affected != 1 {
                return Err(Error::internal(format!(
                    "tenant domain {id} changed during the relocation; run the relocation again"
                )));
            }
        }
        for chunk in ids.chunks(500) {
            domain::Entity::update_many()
                .col_expr(domain::Column::TenantId, Expr::value(to))
                .col_expr(domain::Column::UpdatedAt, Expr::value(now()))
                .filter(domain::Column::Id.is_in(chunk.iter().copied()))
                .exec(connection)
                .await?;
        }
        Ok(found.into_iter().map(|row| row.domain).collect())
    }

    /// `switched` → `cleaned`: deletes at the source the rows that have gone (referenced tables first,
    /// `Custom` tables last), then announces `kernel.relocation_finished` at the receiving tenant (moving the
    /// reporting store…).
    pub(super) async fn clean(&self, row: &Model, runner: &str) -> Result<(), Error> {
        let (prepared, tables, resolved, context) = self.reopen(row).await?;
        let transaction = context.source().begin_write().await?;
        let mut deleted = 0;
        for table in resolved.iter().rev() {
            if let Treatment::Select { mode: Mode::Move, condition } = &table.treatment {
                deleted += tables
                    .delete_matching(&transaction, &table.table, condition.clone())
                    .await
                    .map_err(transfer)?;
            }
        }
        for table in resolved.iter().rev() {
            if let Treatment::Custom(handler) = &table.treatment {
                deleted += handler.clean(&context, &transaction).await?;
            }
        }
        transaction.commit().await?;
        deleted += self
            .clean_blobs(
                &context,
                prepared.source.placement_id,
                prepared_target_placement(&prepared),
            )
            .await?;
        let options = Options::parse(&row.options)?;
        let actor = row.created_by.map_or_else(Actor::system, Actor::user);
        let event = Finished(RelocationEvent::of(row, prepared.kind, &options));
        self.record(prepared.target_id, actor, row.id, &event).await?;
        tracing::info!(relocation_id = %row.id, deleted, "tenant relocation cleaned the source");
        self.set_state(self.control().write(), row, runner, store::CLEANED, Ok).await
    }
}

impl TenantRelocations {
    /// File storage of placement `placement`: its own storage, or the shared storage; `None` when the
    /// installation has no file storage (no module provides one).
    async fn store_of(
        &self,
        placement: Uuid,
    ) -> Result<Option<std::sync::Arc<dyn crate::storage::Storage>>, Error> {
        let own = self.registry.placement_storage(PlacementId::from_uuid(placement)).await?;
        Ok(own.or_else(|| self.registry.shared_storage()))
    }

    /// Files of what travels: keys `tenants/<source>/<module>/<id>/…` (D-259) where `<id>` is a row that goes
    /// (split), or every file of the tenant (merge), together with the new key under `tenants/<target>/`.
    async fn blobs(
        &self,
        context: &RelocationContext,
        source_placement: Uuid,
    ) -> Result<Vec<(String, String)>, Error> {
        let Some(store) = self.store_of(source_placement).await? else { return Ok(Vec::new()) };
        let from = format!("tenants/{}/", context.source_tenant());
        let to = format!("tenants/{}/", context.target_tenant());
        let mut found = Vec::new();
        for key in store.keys_with_prefix(&from).await? {
            let Some(rest) = key.strip_prefix(&from) else { continue };
            let owner = rest.split('/').nth(1).and_then(|id| id.parse::<Uuid>().ok());
            if owner.is_some_and(|id| context.moved(id)) {
                found.push((key.clone(), format!("{to}{rest}")));
            }
        }
        Ok(found)
    }

    /// Copies the files of what travels to the target's storage (overwriting files with the same key). Returns the file count.
    async fn copy_blobs(
        &self,
        context: &RelocationContext,
        source_placement: Uuid,
        target_placement: Uuid,
    ) -> Result<u64, Error> {
        let blobs = self.blobs(context, source_placement).await?;
        let (Some(from), Some(to)) =
            (self.store_of(source_placement).await?, self.store_of(target_placement).await?)
        else {
            return Ok(0);
        };
        for (key, moved) in &blobs {
            // File biến mất giữa lúc liệt kê và lúc đọc (khách không phục vụ, chỉ job dọn xoá):
            // bỏ qua.
            let Some(file) = from.get(key).await? else { continue };
            to.put(moved, file.content).await?;
        }
        Ok(blobs.len() as u64)
    }

    /// Deletes at the source the files already copied to the target (only files the target already has).
    async fn clean_blobs(
        &self,
        context: &RelocationContext,
        source_placement: Uuid,
        target_placement: Uuid,
    ) -> Result<u64, Error> {
        let blobs = self.blobs(context, source_placement).await?;
        let (Some(from), Some(to)) =
            (self.store_of(source_placement).await?, self.store_of(target_placement).await?)
        else {
            return Ok(0);
        };
        let mut deleted = 0;
        for (key, moved) in &blobs {
            if to.get(moved).await?.is_some() {
                from.delete(key).await?;
                deleted += 1;
            }
        }
        Ok(deleted)
    }
}

/// Placement of the receiving tenant (split: the chosen placement; merge: the receiving tenant's placement).
fn prepared_target_placement(prepared: &Prepared) -> Uuid {
    prepared.placement.id
}

/// Source and target databases of the run (checked: openable, fully migrated, different).
fn databases(
    prepared: &Prepared,
) -> Result<(crate::database::Database, crate::database::Database), Error> {
    if let Some(databases) = &prepared.databases {
        return Ok(databases.clone());
    }
    let reasons: Vec<String> = prepared
        .blockers
        .iter()
        .map(|error| format!("{}: {}", error.code(), error.message()))
        .collect();
    Err(Error::internal(format!(
        "the databases of the relocation cannot be used: {}",
        reasons.join("; ")
    )))
}

/// Overrides a row before writing it to the target: the new tenant, the parent node of the root (tables that
/// mirror the organization tree), and secrets re-encrypted for the new tenant.
fn rewrite(
    row: &mut Row,
    context: &RelocationContext,
    table: &Resolved,
    sealed: &[super::SealedColumn],
    sealer: &Sealer,
) -> Result<(), TransferError> {
    let failed = |error: Error| TransferError::new(format!("`{}`: {error}", table.table));
    row.set("tenant_id", Value::Uuid(Some(context.target_tenant().as_uuid())));
    if let Some((id, parent)) = table.org_tree {
        match context.kind() {
            RelocationKind::Split if row.uuid(id) == Some(context.org().as_uuid()) => {
                row.set(parent, Value::Uuid(None));
            }
            RelocationKind::Merge if row.uuid(id) == Some(context.source_tenant().as_uuid()) => {
                row.set(parent, Value::Uuid(Some(context.org().as_uuid())));
            }
            _ => {}
        }
    }
    for column in sealed {
        let Some(value) = row.text(column.column).map(str::to_owned) else { continue };
        if !is_sealed(&value) {
            continue;
        }
        let key = match row.get(column.row_column) {
            Some(found) => crate::transfer::canonical(found).unwrap_or_default(),
            None => String::new(),
        };
        let plain =
            sealer.open(context.source_tenant(), column.purpose, &key, &value).map_err(failed)?;
        let again =
            sealer.seal(context.target_tenant(), column.purpose, &key, &plain).map_err(failed)?;
        row.set(column.column, Value::String(Some(again)));
    }
    Ok(())
}

struct Entity;

impl Entity {
    /// Writes `options` of run `id`.
    async fn update_options<C: sea_orm::ConnectionTrait>(
        connection: &C,
        id: Uuid,
        options: &Options,
    ) -> Result<(), Error> {
        store::Entity::update_many()
            .col_expr(Column::Options, Expr::value(options.json()?))
            .filter(Column::Id.eq(id))
            .exec(connection)
            .await?;
        Ok(())
    }
}

// Độ "mạnh" của một dòng tên miền khi hai khách gộp cùng có tên miền đó.
fn claim_rank(status: &str) -> u8 {
    match status {
        "verified" => 3,
        "pending" => 2,
        _ => 1,
    }
}
