//! Các bước của một lượt tách / gộp. Mỗi bước chạy lại được; không có transaction xuyên hai
//! database — chép là MỘT transaction ở đích, chuyển là một transaction ở database điều khiển, dọn
//! là một transaction ở nguồn và chỉ xoá dòng đã có ở đích.

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

/// Nội dung của event `kernel.relocation_*`.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct RelocationEvent {
    relocation_id: Uuid,
    kind: RelocationKind,
    source_tenant_id: Uuid,
    target_tenant_id: Option<Uuid>,
    org_id: Option<Uuid>,
    /// Khách gốc của cây chứa dữ liệu trước và sau (dời kho báo cáo gộp).
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
    /// Đặt `status` cho khách `ids` khi khách đang ở một trong `from`.
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

    /// Khách bị khoá trong lượt: khách nguồn, và khách nhận khi gộp.
    fn frozen_tenants(row: &Model, kind: RelocationKind) -> Vec<Uuid> {
        let mut ids = vec![row.source_tenant_id];
        if kind == RelocationKind::Merge {
            ids.extend(row.target_tenant_id);
        }
        ids
    }

    /// Ghi event `event` cho khách `tenant` vào database của nó (ngoài transaction khác).
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

    /// `planned` → `frozen`: khách thành `maintenance` (request nhận 503, worker vẫn chạy), chờ
    /// outbox / job đến hạn của khách chạy hết, rồi `moving` (worker và bộ phát dừng việc của khách),
    /// chờ máy chủ khác thấy và việc đang chạy xong.
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

    /// Dừng lượt trước khi đổi gì: khách phục vụ lại, lượt `failed`.
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

    /// Kiểm lại, bảng và ngữ cảnh của một lượt đã lưu.
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

    /// `frozen` → `copied`: (tách) tạo khách mới `provisioning`; chép trong MỘT transaction ở đích
    /// mọi bảng theo chính sách, giữ khoá chính, đổi `tenant_id`; rồi bảng `Custom`.
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

    /// Khách mới khi tách (`provisioning`), nếu chưa có.
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

    /// `copied` → `verified`: mọi dòng đi / được chép (theo khoá chính) đã có ở đích.
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

    /// Khách gốc của cây chứa khách `tenant`.
    async fn root(&self, tenant: Uuid) -> Result<Uuid, Error> {
        let above = self.registry.tenant_ancestors(TenantId::from_uuid(tenant)).await?;
        Ok(above.last().map_or(tenant, TenantId::as_uuid))
    }

    /// `verified` → `switched`: một transaction ở database điều khiển — khách mới hoạt động / khách
    /// bị gộp thành `merged`, tên miền gắn nút trong nhánh và khách con đi theo; khách nguồn mở lại.
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

    /// Tên miền của khách `from` (chỉ các tên miền gắn nút trong `branch`, nếu có) sang khách `to`.
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
        let ids: Vec<Uuid> = found.iter().map(|row| row.id).collect();
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

    /// `switched` → `cleaned`: xoá ở nguồn dòng đã đi (bảng tham chiếu trước, bảng `Custom` sau
    /// cùng), rồi báo `kernel.relocation_finished` ở khách nhận (dời kho báo cáo…).
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
    /// Nơi lưu file của placement `placement`: nơi lưu riêng của nó, hay nơi lưu chung; `None` khi
    /// bản cài không có nơi lưu file (không module nào cung cấp).
    async fn store_of(
        &self,
        placement: Uuid,
    ) -> Result<Option<std::sync::Arc<dyn crate::storage::Storage>>, Error> {
        let own = self.registry.placement_storage(PlacementId::from_uuid(placement)).await?;
        Ok(own.or_else(|| self.registry.shared_storage()))
    }

    /// File của thứ đi theo: khoá `tenants/<nguồn>/<module>/<id>/…` (D-259) mà `<id>` là một dòng
    /// đi (tách) — hay mọi file của khách (gộp) — kèm khoá mới dưới `tenants/<đích>/`.
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

    /// Chép file của thứ đi theo sang nơi lưu của đích (đè file cùng khoá). Trả số file.
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

    /// Xoá ở nguồn file đã chép sang đích (chỉ file mà đích đã có).
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

/// Placement của khách nhận (tách: placement đã chọn; gộp: placement của khách nhận).
fn prepared_target_placement(prepared: &Prepared) -> Uuid {
    prepared.placement.id
}

/// Database nguồn và đích của lượt (đã kiểm mở được, đủ migration, khác nhau).
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

/// Ghi đè một dòng trước khi ghi vào đích: khách mới, nút cha của gốc (bảng soi cây tổ chức),
/// secret mã hoá lại cho khách mới.
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
    /// Ghi `options` của lượt `id`.
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
