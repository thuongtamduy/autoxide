//! Pre-move checks and planning (dry run).

use super::files::tenant_prefix;
use super::*;

impl TenantMoves {
    /// Checks moving tenant `tenant` to placement `to`: every blocking reason (it does not stop at the first)
    /// and the number of rows that would be copied. A missing tenant or placement: an error.
    pub(super) async fn check(&self, tenant: &str, to: &str) -> Result<Checked, Error> {
        let row = self.tenant_row(tenant).await?;
        let placements = self.placements().await?;
        let target = placements
            .iter()
            .find(|placement| placement.name == to)
            .cloned()
            .ok_or_else(|| errors::tenant_placement_not_found(to))?;
        let name_of = |id: Uuid| {
            placements
                .iter()
                .find(|placement| placement.id == id)
                .map_or_else(|| id.to_string(), |placement| placement.name.clone())
        };
        let mut blockers = Vec::new();
        let mut resume = None;
        if row.id == DEFAULT_TENANT.as_uuid() {
            blockers.push(errors::tenant_move_platform());
        }
        let unfinished = self.unfinished(row.id).await?;
        let from_id = match &unfinished {
            Some(moving) => {
                let now = now();
                if moving.runner_until.is_some_and(|until| until > now) {
                    blockers.push(errors::tenant_move_running(&name_of(moving.to_placement_id)));
                } else if moving.to_placement_id != target.id {
                    blockers.push(errors::tenant_move_unfinished(&name_of(moving.to_placement_id)));
                } else {
                    resume = Some(moving.clone());
                }
                moving.from_placement_id
            }
            None => {
                if target.id == row.placement_id {
                    blockers.push(errors::tenant_move_same_placement(to));
                }
                if row.status != TenantStatus::Active.as_str() {
                    blockers.push(errors::tenant_move_not_active(&row.status));
                }
                row.placement_id
            }
        };
        let source = placements
            .iter()
            .find(|placement| placement.id == from_id)
            .cloned()
            .ok_or_else(|| errors::tenant_placement_not_found(&from_id.to_string()))?;
        if target.id != MAIN_PLACEMENT.as_uuid() && target.schema_state != "ready" {
            blockers.push(errors::tenant_placement_behind(to));
        }
        // Khách buộc lưu dữ liệu ở một nước: chỉ dời tới placement ở đúng nước đó (D-360).
        if let Err(error) = super::super::residency::check_target(
            row.data_residency.as_deref(),
            target.country_code.as_deref(),
        ) {
            blockers.push(error);
        }
        let mut tables = Vec::new();
        let source_db = self.database(&source).await;
        let target_db = self.database(&target).await;
        match (source_db, target_db) {
            (Ok(source_db), Ok(target_db)) => {
                // Lượt dời chỉ chép bảng của module đang bật (bước 70 P1): còn dữ liệu của module
                // đang tắt thì dừng, không để nó ở lại database nguồn.
                let left_behind =
                    self.setup.inactive_with_data(&source_db, TenantId::from_uuid(row.id)).await?;
                if !left_behind.is_empty() {
                    blockers.push(errors::tenant_move_modules_off(&left_behind.join(", ")));
                }
                let schema = self.schema_problems(&source, &source_db, &target, &target_db).await?;
                let schema_ok = schema.is_empty();
                blockers.extend(schema);
                if schema_ok {
                    let setup = self.setup.get()?;
                    let found = TenantTables::read(
                        &source_db,
                        &target_db,
                        &setup.modules,
                        &setup.control_plane,
                    )
                    .await
                    .map_err(transfer)?;
                    let tenant = TenantId::from_uuid(row.id);
                    tables = found
                        .count(&source_db, tenant)
                        .await
                        .map_err(transfer)?
                        .into_iter()
                        .map(TenantTableRows::from)
                        .collect();
                }
            }
            (source_db, target_db) => {
                blockers.extend(source_db.err());
                blockers.extend(target_db.err());
            }
        }
        let mut files = 0;
        match self.file_stores(&source, &target).await {
            Ok(Some((from_store, _))) => {
                match from_store.keys_with_prefix(&tenant_prefix(row.id)).await {
                    Ok(keys) => files = keys.len() as u64,
                    Err(error) => blockers.push(errors::tenant_move_storage_unavailable(
                        &source.name,
                        &error.to_string(),
                    )),
                }
            }
            Ok(None) => {}
            Err(error) => blockers.push(error),
        }
        Ok(Checked { tenant: row, from: source, to: target, blockers, resume, tables, files })
    }

    /// The file store of placement `placement`: its own store, or the shared store. Comes with the
    /// configuration of the own store (`None`: the shared store) so two placements can be compared.
    pub(super) async fn file_store(
        &self,
        placement: &placement::Model,
    ) -> Result<(Option<crate::storage::StorageLocation>, Arc<dyn Storage>), Error> {
        let unavailable = |error: Error| {
            errors::tenant_move_storage_unavailable(&placement.name, &error.to_string())
        };
        let id = PlacementId::from_uuid(placement.id);
        match (&placement.storage_sealed, id == MAIN_PLACEMENT) {
            (Some(sealed), false) => {
                let location =
                    self.registry.storage_location(id, sealed).await.map_err(unavailable)?;
                let store = self.registry.placement_storage(id).await.map_err(unavailable)?;
                let store = store.ok_or_else(|| {
                    unavailable(Error::internal("the placement no longer has its own storage"))
                })?;
                Ok((Some(location), store))
            }
            _ => {
                let store = self.registry.shared_storage().ok_or_else(|| {
                    unavailable(Error::internal("no module provides the file storage"))
                })?;
                Ok((None, store))
            }
        }
    }

    /// The file stores of `from` and `to` when the two placements use two different stores; `None`: the same
    /// store (the same shared store, or own stores that point to the same place), so no file needs to be
    /// copied or deleted.
    pub(super) async fn file_stores(
        &self,
        from: &placement::Model,
        to: &placement::Model,
    ) -> Result<Option<(Arc<dyn Storage>, Arc<dyn Storage>)>, Error> {
        let own = |placement: &placement::Model| {
            placement.storage_sealed.is_some() && placement.id != MAIN_PLACEMENT.as_uuid()
        };
        if !own(from) && !own(to) {
            return Ok(None);
        }
        let (from_location, from_store) = self.file_store(from).await?;
        let (to_location, to_store) = self.file_store(to).await?;
        if let (Some(a), Some(b)) = (&from_location, &to_location)
            && a.same_place(b)
        {
            return Ok(None);
        }
        Ok(Some((from_store, to_store)))
    }

    /// The reason two databases cannot be copied to each other: the same database, or missing migrations.
    pub(super) async fn schema_problems(
        &self,
        source: &placement::Model,
        source_db: &Database,
        target: &placement::Model,
        target_db: &Database,
    ) -> Result<Vec<Error>, Error> {
        let mut problems = Vec::new();
        if same_database(source_db, target_db).await.map_err(transfer)? {
            problems.push(errors::tenant_move_same_database(&target.name));
        }
        let migrator = &self.setup.get()?.migrator;
        for (placement, database) in [(source, source_db), (target, target_db)] {
            let pending = migrator.pending(database).await.map_err(|error| {
                errors::tenant_move_unreachable(&placement.name, &error.to_string())
            })?;
            if !pending.is_empty() {
                problems.push(errors::tenant_move_schema_differs(&placement.name, pending.len()));
            }
        }
        Ok(problems)
    }

    /// The plan for moving tenant `tenant` (ID or slug) to placement `to`, changing nothing: the blocking
    /// reasons (sentences in `language`) and the number of rows that would be copied.
    pub async fn plan(
        &self,
        tenant: &str,
        to: &str,
        keep_source: bool,
        language: Language,
    ) -> Result<TenantMovePlan, Error> {
        let checked = self.check(tenant, to).await?;
        let blocking: Vec<TenantMoveBlocker> = checked
            .blockers
            .iter()
            .map(|error| TenantMoveBlocker {
                code: error.code().to_owned(),
                message: self.translations.message(error, language),
            })
            .collect();
        Ok(TenantMovePlan {
            tenant_id: checked.tenant.id,
            tenant: checked.tenant.slug,
            from: checked.from.name,
            to: checked.to.name,
            keep_source: checked.resume.as_ref().map_or(keep_source, |moving| moving.keep_source),
            ready: blocking.is_empty(),
            blocking,
            resumes: checked.resume.map(|moving| moving.id),
            rows: checked.tables.iter().map(|table| table.rows).sum(),
            tables: checked.tables,
            files: checked.files,
            settle_seconds: self.settle.as_secs(),
        })
    }
}
