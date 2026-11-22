//! Kiểm một lượt tách / gộp trước khi chạy: đầu vào, trạng thái các khách, nút, placement,
//! database — mọi lý do chặn (không dừng ở lý do đầu) — rồi lập kế hoạch các bảng.

use std::collections::HashSet;

use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Query};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
use uuid::Uuid;

use super::plan::{self, Report};
use super::policy::Resolved;
use super::run::TenantRelocations;
use super::store::{self, Column, Entity, Model, Options};
use super::{RelocationContext, RelocationKind, errors};
use crate::database::Database;
use crate::error::{Error, FieldError};
use crate::tenancy::admin::{placement, tenant};
use crate::tenancy::{
    DEFAULT_TENANT, MAIN_PLACEMENT, OrgId, PlacementId, TenantId, TenantStatus, errors as tenancy,
};
use crate::transfer::{TenantTables, TransferError, same_database};

fn transfer(error: TransferError) -> Error {
    Error::internal(error.to_string())
}

/// Tham số đã chuẩn hoá của một lượt.
#[derive(Debug, Clone)]
pub(crate) struct Spec {
    pub(crate) kind: RelocationKind,
    /// Khách có dữ liệu đi (ID hay slug).
    pub(crate) tenant: String,
    pub(crate) org: Option<Uuid>,
    pub(crate) into: Option<String>,
    pub(crate) under_org: Option<Uuid>,
    pub(crate) options: Options,
}

impl Spec {
    /// Lỗi đầu vào (gom một lần, D-083).
    pub(crate) fn check(&self, need_placement: bool) -> Result<(), Error> {
        let mut problems = Vec::new();
        if self.options.reason.trim().is_empty() {
            problems.push(FieldError::new(
                "reason",
                "required",
                "is required: say why the data moves between tenants (contract, consent)",
            ));
        }
        match self.kind {
            RelocationKind::Split => {
                if self.org.is_none() {
                    problems.push(FieldError::new("org_id", "required", "is required to split"));
                }
                match &self.options.slug {
                    Some(slug) if crate::tenancy::admin::is_slug(slug) => {}
                    _ => problems.push(FieldError::new(
                        "slug",
                        "invalid",
                        "is required to split: 1 to 63 lower-case letters, digits and `-`",
                    )),
                }
                if self.options.name.as_deref().is_none_or(|name| name.trim().is_empty()) {
                    problems.push(FieldError::new("name", "required", "is required to split"));
                }
                if need_placement && self.options.placement.is_none() {
                    problems.push(FieldError::new("placement", "required", "is required to split"));
                }
            }
            RelocationKind::Merge => {
                if self.into.is_none() {
                    problems.push(FieldError::new("into", "required", "is required to merge"));
                }
                if self.under_org.is_none() {
                    problems.push(FieldError::new("under_org", "required", "is required to merge"));
                }
            }
        }
        if problems.is_empty() { Ok(()) } else { Err(Error::validation(problems)) }
    }
}

/// Kết quả kiểm.
pub(crate) struct Prepared {
    pub(crate) kind: RelocationKind,
    pub(crate) source: tenant::Model,
    pub(crate) target_id: Uuid,
    pub(crate) target_slug: String,
    pub(crate) org: OrgId,
    pub(crate) placement: placement::Model,
    pub(crate) blockers: Vec<Error>,
    /// Database nguồn và đích, khi mở được và đủ migration.
    pub(crate) databases: Option<(Database, Database)>,
}

impl TenantRelocations {
    pub(super) async fn tenant_row(&self, id_or_slug: &str) -> Result<tenant::Model, Error> {
        let select = match id_or_slug.parse::<Uuid>() {
            Ok(id) => tenant::Entity::find_by_id(id),
            Err(_) => tenant::Entity::find().filter(tenant::Column::Slug.eq(id_or_slug)),
        };
        select
            .one(self.control().write())
            .await?
            .ok_or_else(|| tenancy::tenant_not_found(id_or_slug))
    }

    pub(super) async fn placement_row(&self, id: Uuid) -> Result<placement::Model, Error> {
        placement::Entity::find_by_id(id)
            .one(self.control().write())
            .await?
            .ok_or_else(|| tenancy::tenant_placement_not_found(&id.to_string()))
    }

    pub(super) async fn database(&self, placement: &placement::Model) -> Result<Database, Error> {
        self.registry
            .placement_database(PlacementId::from_uuid(placement.id))
            .await
            .map_err(|error| tenancy::tenant_move_unreachable(&placement.name, &error.message()))
    }

    /// Lượt chưa xong của khách `tenant` (nguồn hay đích), trừ lượt `except`.
    async fn unfinished(&self, tenant: Uuid, except: Option<Uuid>) -> Result<Option<Uuid>, Error> {
        let rows = Entity::find()
            .filter(
                sea_orm::Condition::any()
                    .add(Column::SourceTenantId.eq(tenant))
                    .add(Column::TargetTenantId.eq(tenant)),
            )
            .filter(Column::State.is_in(store::UNFINISHED))
            .all(self.control().write())
            .await?;
        if let Some(row) = rows.into_iter().find(|row| Some(row.id) != except) {
            return Ok(Some(row.id));
        }
        // Lượt dời khách giữa placement (bước 33) đang dở cũng chặn.
        let select = Query::select()
            .column(Alias::new("id"))
            .from(Alias::new("kernel_tenant_moves"))
            .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant))
            .and_where(Expr::col(Alias::new("state")).is_in(["copying", "copied", "switched"]))
            .to_owned();
        let row = self.control().write().query_one(&select).await?;
        Ok(match row {
            Some(row) => Some(row.try_get::<Uuid>("", "id")?),
            None => None,
        })
    }

    /// Kiểm lượt `spec` (`resume`: lượt đã lưu đang chạy tiếp — bỏ qua trạng thái do chính nó đặt).
    pub(super) async fn prepare(
        &self,
        spec: &Spec,
        resume: Option<&Model>,
    ) -> Result<Prepared, Error> {
        let source = self.tenant_row(&spec.tenant).await?;
        let mut blockers = Vec::new();
        let resuming = resume.map(|row| row.id);
        if source.id == DEFAULT_TENANT.as_uuid() {
            blockers.push(errors::platform());
        }
        if resume.is_none() && source.status != TenantStatus::Active.as_str() {
            blockers.push(errors::not_active(&source.slug, &source.status));
        }
        if let Some(id) = self.unfinished(source.id, resuming).await? {
            blockers.push(errors::in_progress(&id.to_string()));
        }
        let source_placement = self.placement_row(source.placement_id).await?;
        let source_db = self.database(&source_placement).await;
        // Tách / gộp chỉ chuyển bảng của module đang bật (bước 70 P1): còn dữ liệu của module đang
        // tắt thì dừng, không để nó ở lại khách nguồn.
        if let Ok(database) = &source_db {
            let left_behind =
                self.setup.inactive_with_data(database, TenantId::from_uuid(source.id)).await?;
            if !left_behind.is_empty() {
                blockers.push(tenancy::tenant_move_modules_off(&left_behind.join(", ")));
            }
        }
        let (target_id, target_slug, org, target_placement) = match spec.kind {
            RelocationKind::Split => {
                let org = spec.org.unwrap_or_default();
                let name = spec.options.placement.clone().unwrap_or_default();
                let found = placement::Entity::find()
                    .filter(placement::Column::Name.eq(name.as_str()))
                    .one(self.control().write())
                    .await?;
                let target_placement = match (found, resume) {
                    (Some(found), _) => found,
                    (None, Some(row)) => {
                        self.placement_row(row.placement_id.unwrap_or_default()).await?
                    }
                    (None, None) => return Err(tenancy::tenant_placement_not_found(&name)),
                };
                if resume.is_none() {
                    if tenant::Entity::find_by_id(org).one(self.control().write()).await?.is_some()
                    {
                        blockers.push(errors::tenant_exists(&org.to_string()));
                    }
                    let slug = spec.options.slug.clone().unwrap_or_default();
                    let taken = tenant::Entity::find()
                        .filter(tenant::Column::Slug.eq(slug.as_str()))
                        .one(self.control().write())
                        .await?;
                    if taken.is_some() {
                        blockers.push(tenancy::tenant_slug_taken(&slug));
                    }
                }
                if let Err(error) = crate::tenancy::residency::check_target(
                    source.data_residency.as_deref(),
                    target_placement.country_code.as_deref(),
                ) {
                    blockers.push(error);
                }
                if let Ok(database) = &source_db
                    && (org == source.id
                        || !crate::tenancy::OrgTree::is_within(
                            database.write(),
                            TenantId::from_uuid(source.id),
                            OrgId::from_uuid(org),
                            &[OrgId::from_uuid(source.id)],
                        )
                        .await?)
                {
                    blockers.push(errors::org_invalid(&org.to_string(), &source.slug));
                }
                (org, spec.options.slug.clone().unwrap_or_default(), org, target_placement)
            }
            RelocationKind::Merge => {
                let target = self.tenant_row(spec.into.as_deref().unwrap_or_default()).await?;
                let under = spec.under_org.unwrap_or_default();
                if target.id == DEFAULT_TENANT.as_uuid() {
                    blockers.push(errors::platform());
                }
                if resume.is_none() && target.status != TenantStatus::Active.as_str() {
                    blockers.push(errors::not_active(&target.slug, &target.status));
                }
                if let Some(id) = self.unfinished(target.id, resuming).await? {
                    blockers.push(errors::in_progress(&id.to_string()));
                }
                let below =
                    self.registry.tenant_descendants(TenantId::from_uuid(source.id)).await?;
                if target.id == source.id
                    || below.iter().any(|child| child.id.as_uuid() == target.id)
                {
                    blockers.push(errors::cycle(&target.slug));
                }
                let target_placement = self.placement_row(target.placement_id).await?;
                if let Ok(database) = self.database(&target_placement).await {
                    let within = crate::tenancy::OrgTree::is_within(
                        database.write(),
                        TenantId::from_uuid(target.id),
                        OrgId::from_uuid(under),
                        &[OrgId::from_uuid(target.id)],
                    )
                    .await?;
                    if !within {
                        blockers.push(errors::org_invalid(&under.to_string(), &target.slug));
                    }
                }
                (target.id, target.slug, under, target_placement)
            }
        };
        if target_placement.id != MAIN_PLACEMENT.as_uuid()
            && target_placement.schema_state != "ready"
        {
            blockers.push(tenancy::tenant_placement_behind(&target_placement.name));
        }
        let target_db = self.database(&target_placement).await;
        let databases = match (source_db, target_db) {
            (Ok(source_db), Ok(target_db)) => {
                let mut ready = true;
                if same_database(&source_db, &target_db).await.map_err(transfer)? {
                    blockers.push(errors::same_database(&target_placement.name));
                    ready = false;
                }
                for (placement, database) in
                    [(&source_placement, &source_db), (&target_placement, &target_db)]
                {
                    let pending = self.setup.pending(database).await.map_err(|error| {
                        tenancy::tenant_move_unreachable(&placement.name, &error.to_string())
                    })?;
                    if pending > 0 {
                        blockers.push(errors::schema_differs(&placement.name, pending));
                        ready = false;
                    }
                }
                ready.then_some((source_db, target_db))
            }
            (source_db, target_db) => {
                blockers.extend(source_db.err());
                blockers.extend(target_db.err());
                None
            }
        };
        Ok(Prepared {
            kind: spec.kind,
            source,
            target_id,
            target_slug,
            org: OrgId::from_uuid(org),
            placement: target_placement,
            blockers,
            databases,
        })
    }

    /// Bảng của hai database và việc của từng bảng.
    pub(super) async fn resolve(
        &self,
        prepared: &Prepared,
        source: &Database,
        target: &Database,
    ) -> Result<(TenantTables, Vec<Resolved>), Error> {
        let (modules, control_plane) = self.setup.tables()?;
        let tables =
            TenantTables::read(source, target, &modules, &control_plane).await.map_err(transfer)?;
        let resolved = self.setup.policies()?.resolve(
            &tables,
            prepared.kind,
            TenantId::from_uuid(prepared.source.id),
            prepared.org,
        )?;
        Ok((tables, resolved))
    }

    /// Ngữ cảnh cho bảng `Custom`, kèm thứ đi theo (tách).
    pub(super) async fn context(
        &self,
        id: Uuid,
        prepared: &Prepared,
        source: &Database,
        target: &Database,
        tables: &TenantTables,
        resolved: &[Resolved],
    ) -> Result<RelocationContext, Error> {
        let source_tenant = TenantId::from_uuid(prepared.source.id);
        let moved = match prepared.kind {
            RelocationKind::Split => {
                let branch = plan::branch(source, source_tenant, prepared.org).await?;
                plan::moved_ids(tables, resolved, source, &branch).await?
            }
            RelocationKind::Merge => HashSet::new(),
        };
        Ok(RelocationContext::new(
            id,
            prepared.kind,
            source_tenant,
            TenantId::from_uuid(prepared.target_id),
            source.clone(),
            target.clone(),
            prepared.org,
            moved,
            tables.clone(),
        ))
    }

    /// Kế hoạch các bảng (khi hai database dùng được).
    pub(super) async fn report(&self, prepared: &Prepared) -> Result<Option<Report>, Error> {
        let Some((source, target)) = &prepared.databases else { return Ok(None) };
        let (tables, resolved) = self.resolve(prepared, source, target).await?;
        let context =
            self.context(Uuid::nil(), prepared, source, target, &tables, &resolved).await?;
        Ok(Some(plan::report(&tables, &resolved, &context).await?))
    }
}
