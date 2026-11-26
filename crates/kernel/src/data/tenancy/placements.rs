//! Placement: where a tenant's data lives (spec 33 "Connection registry, jobs, events, migrations across
//! many databases", D-261): add (the connection URL is stored encrypted), list, check migrations
//! (`schema_state`), migrate every placement.
//!
//! - Every database has the same schema (D-253 item 2): a placement runs exactly the migration set of the
//!   control database. Data upgrades on a placement use the root key of the control database
//!   ([`Migrator::keyed_on`]); the module encrypts the secrets of every tenant with that key.
//! - `schema_state`: `ready` (all migrations applied), `behind` (migrations still pending; tenants on it get
//!   `tenant_maintenance` and their jobs wait), `migrating` (`migrate up --all` is running). A re-check does
//!   not overwrite `migrating`; every state-changing command writes only when the state has not been changed
//!   by another party (compared with the value just read).
//! - The `main` placement (the control database) has no state of its own: `serve` requires it to have all
//!   migrations before starting.

use crate::time::Timestamp;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, Set,
};
use std::sync::Arc;
use uuid::Uuid;

use super::admin::placement;
use super::{
    DatabaseRegistry, MAIN_PLACEMENT, NO_TENANT, PLACEMENT_READ_URL, PLACEMENT_STORAGE,
    PLACEMENT_URL, PlacementId, PlacementKind, errors,
};
use crate::config::{DatabaseBackend, DatabaseUrl};
use crate::error::{Error, FieldError};
use crate::migration::{MigrationLock, MigrationState, Migrator};
use crate::storage::StorageLocation;

/// Schema state of a placement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchemaState {
    /// All migrations applied.
    Ready,
    /// Migrations still pending (or cannot connect): tenants on it receive `503`.
    Behind,
    /// `autoxide migrate up --all` is running on it.
    Migrating,
}

impl SchemaState {
    pub fn as_str(self) -> &'static str {
        match self {
            SchemaState::Ready => "ready",
            SchemaState::Behind => "behind",
            SchemaState::Migrating => "migrating",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "ready" => Some(SchemaState::Ready),
            "behind" => Some(SchemaState::Behind),
            "migrating" => Some(SchemaState::Migrating),
            _ => None,
        }
    }
}

/// A new placement. The URL contains a password: stored encrypted only, never printed (the `Debug` of
/// [`DatabaseUrl`] masks the password).
#[derive(Clone, Debug)]
pub struct NewPlacement {
    name: String,
    kind: PlacementKind,
    url: DatabaseUrl,
    read_url: Option<DatabaseUrl>,
}

impl NewPlacement {
    /// Placement named `name` (lowercase, digits, `-`; at most 63 characters), kind `kind`, at `url`.
    pub fn new(name: impl Into<String>, kind: PlacementKind, url: DatabaseUrl) -> Self {
        NewPlacement { name: name.into(), kind, url, read_url: None }
    }

    /// Read replica (Postgres) for read commands.
    #[must_use]
    pub fn read_url(mut self, url: DatabaseUrl) -> Self {
        self.read_url = Some(url);
        self
    }
}

/// Result of checking the migrations of one placement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlacementCheck {
    pub id: PlacementId,
    pub name: String,
    pub state: SchemaState,
    /// Number of migrations not yet applied.
    pub pending: usize,
    /// Could not connect or could not read the migration state (then `state` is `behind`).
    pub error: Option<String>,
}

/// Migration of one database: `main` (the control database) or a placement.
#[derive(Clone, Debug)]
pub struct PlacementMigrations {
    pub name: String,
    pub migrations: Vec<MigrationState>,
}

/// Placement management (the `autoxide placement` command, `autoxide migrate --all`, the periodic check of
/// `serve`). Cheap to clone.
#[derive(Clone)]
pub struct Placements {
    registry: DatabaseRegistry,
    migrator: Arc<Migrator>,
}

impl std::fmt::Debug for Placements {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Placements").finish_non_exhaustive()
    }
}

impl Placements {
    /// Migrator of every module (scheduled backup, step 39).
    pub(crate) fn migrator(&self) -> &Migrator {
        &self.migrator
    }
}

fn now() -> Timestamp {
    Timestamp::now()
}

impl Placements {
    /// `migrator`: the migrations of every module (`Modules::migrator`).
    pub fn new(registry: DatabaseRegistry, migrator: Migrator) -> Self {
        let migrator = Arc::new(migrator.keyed_on(registry.control()));
        Placements { registry, migrator }
    }

    pub(super) fn registry(&self) -> &DatabaseRegistry {
        &self.registry
    }

    async fn rows(&self) -> Result<Vec<placement::Model>, Error> {
        Ok(placement::Entity::find()
            .filter(placement::Column::Id.ne(MAIN_PLACEMENT.as_uuid()))
            .order_by_asc(placement::Column::Name)
            .all(self.registry.control().read())
            .await?)
    }

    /// Every placement (`main` included), by name. Never includes the URL.
    pub async fn list(&self) -> Result<Vec<super::Placement>, Error> {
        let rows = placement::Entity::find()
            .order_by_asc(placement::Column::Name)
            .all(self.registry.control().read())
            .await?;
        let mut all = Vec::with_capacity(rows.len());
        for row in rows {
            all.push(super::admin::describe_placement(&self.registry, row).await);
        }
        Ok(all)
    }

    /// Sets (`Some`) or removes (`None`, the tenant's files go back to the shared storage) the own file
    /// storage of placement `name`. Stored encrypted (purpose [`PLACEMENT_STORAGE`]); never returns the secret
    /// key. A placement that has tenants is refused (`tenant_placement_storage_in_use`: their files are in
    /// the old storage) unless `force`, for when the new storage already holds exactly those files (e.g. an
    /// access key change). Does not try to connect: a configuration error appears on the first file read /
    /// write (`autoxide doctor`).
    pub async fn set_storage(
        &self,
        name: &str,
        location: Option<StorageLocation>,
        force: bool,
    ) -> Result<super::Placement, Error> {
        let location = location.map(StorageLocation::checked).transpose()?;
        let row = placement::Entity::find()
            .filter(placement::Column::Name.eq(name))
            .one(self.registry.control().read())
            .await?
            .ok_or_else(|| errors::tenant_placement_not_found(name))?;
        if row.id == MAIN_PLACEMENT.as_uuid() {
            return Err(Error::validation(vec![FieldError::new(
                "placement",
                "main",
                "the placement `main` keeps its files where the `files` module settings say",
            )]));
        }
        let id = PlacementId::from_uuid(row.id);
        if !force {
            let tenants = super::admin::tenant::Entity::find()
                .filter(super::admin::tenant::Column::PlacementId.eq(row.id))
                .count(self.registry.control().read())
                .await?;
            if tenants > 0 {
                return Err(errors::tenant_placement_storage_in_use(name, tenants));
            }
        }
        let sealed = match &location {
            Some(location) => {
                let json =
                    zeroize::Zeroizing::new(serde_json::to_vec(location).map_err(Error::internal)?);
                let sealer = self.registry.sealer().await?;
                Some(sealer.seal(NO_TENANT, PLACEMENT_STORAGE, &id.to_string(), &json)?)
            }
            None => None,
        };
        placement::Entity::update_many()
            .col_expr(placement::Column::StorageSealed, Expr::value(sealed))
            .col_expr(placement::Column::UpdatedAt, Expr::value(now()))
            .filter(placement::Column::Id.eq(row.id))
            .exec(self.registry.control().write())
            .await?;
        self.registry.invalidate_placement(id);
        let kind = match &location {
            Some(StorageLocation::Local { .. }) => "local",
            Some(StorageLocation::S3(_)) => "s3",
            None => "shared",
        };
        tracing::info!(placement = %name, storage = kind, "placement file storage changed");
        let row = placement::Entity::find_by_id(row.id)
            .one(self.registry.control().write())
            .await?
            .ok_or_else(|| errors::tenant_placement_not_found(name))?;
        Ok(super::admin::describe_placement(&self.registry, row).await)
    }

    /// Adds a placement: checks the name, tries to connect, stores the URL encrypted, then checks migrations
    /// (an already migrated database is `ready` at once; an empty database is `behind` until
    /// `migrate up --all`).
    pub async fn add(&self, new: NewPlacement) -> Result<PlacementCheck, Error> {
        let name = new.name.trim().to_owned();
        if !super::admin::is_slug(&name) || name == "main" {
            return Err(Error::validation(vec![FieldError::new(
                "name",
                "invalid",
                "must be 1 to 63 lower-case letters, digits and `-`, not starting or ending with \
                 `-`, and not `main`",
            )]));
        }
        let taken = placement::Entity::find()
            .filter(placement::Column::Name.eq(name.as_str()))
            .one(self.registry.control().read())
            .await?;
        if taken.is_some() {
            return Err(errors::tenant_placement_taken(&name));
        }
        // Thử kết nối trước khi lưu: URL sai thì báo ngay, không để khách nhận 503 về sau.
        let probe = self
            .registry
            .open(new.url.expose(), new.read_url.as_ref().map(DatabaseUrl::expose))
            .await
            .map_err(|error| {
                Error::validation(vec![
                    FieldError::new(
                        "url",
                        "unreachable",
                        "cannot connect to the database: {error}",
                    )
                    .with_param("error", error),
                ])
            })?;
        let _ = probe.close().await;
        let id = PlacementId::from_uuid(Uuid::now_v7());
        let sealer = self.registry.sealer().await?;
        let row = id.to_string();
        let url = sealer.seal(NO_TENANT, PLACEMENT_URL, &row, new.url.expose().as_bytes())?;
        let read_url = match &new.read_url {
            Some(read) => {
                Some(sealer.seal(NO_TENANT, PLACEMENT_READ_URL, &row, read.expose().as_bytes())?)
            }
            None => None,
        };
        let backend = match new.url.backend() {
            DatabaseBackend::Postgres => "postgres",
            DatabaseBackend::Sqlite => "sqlite",
        };
        let at = now();
        let model = placement::ActiveModel {
            id: Set(id.as_uuid()),
            name: Set(name.clone()),
            kind: Set(new.kind.as_str().to_owned()),
            backend: Set(backend.to_owned()),
            url_sealed: Set(Some(url)),
            read_url_sealed: Set(read_url),
            storage_sealed: Set(None),
            schema_state: Set(SchemaState::Behind.as_str().to_owned()),
            schema_checked_at: Set(None),
            created_at: Set(at),
            updated_at: Set(at),
            country_code: Set(None),
        };
        // Hai lệnh cùng tên cùng lúc: khoá unique của bảng chặn bên sau.
        let inserted = model.insert(self.registry.control().write()).await.map_err(|error| {
            if matches!(error.sql_err(), Some(sea_orm::SqlErr::UniqueConstraintViolation(_))) {
                errors::tenant_placement_taken(&name)
            } else {
                Error::from(error)
            }
        })?;
        tracing::info!(placement = %name, backend, "placement added");
        self.check_row(inserted).await
    }

    /// Checks the migrations of every placement (except `main`) and updates `schema_state`. Failing to connect
    /// to one placement does not fail the whole command: that placement becomes `behind`, with the error.
    pub async fn check(&self) -> Result<Vec<PlacementCheck>, Error> {
        let mut checks = Vec::new();
        for row in self.rows().await? {
            checks.push(self.check_row(row).await?);
        }
        Ok(checks)
    }

    async fn check_row(&self, row: placement::Model) -> Result<PlacementCheck, Error> {
        let id = PlacementId::from_uuid(row.id);
        let found = match self.registry.placement_database(id).await {
            Ok(database) => {
                self.migrator.pending(&database).await.map_err(|error| error.to_string())
            }
            Err(error) => Err(error.to_string()),
        };
        let (mut state, pending, error) = match found {
            Ok(pending) if pending.is_empty() => (SchemaState::Ready, 0, None),
            Ok(pending) => (SchemaState::Behind, pending.len(), None),
            Err(error) => (SchemaState::Behind, 0, Some(error)),
        };
        // `migrate up --all` đang chạy: để nó tự đặt trạng thái khi xong.
        if row.schema_state == SchemaState::Migrating.as_str() {
            state = SchemaState::Migrating;
        }
        self.set_state(id, &row.schema_state, state).await?;
        if let Some(error) = &error {
            tracing::warn!(placement = %row.name, error = %error, "cannot check the placement");
        }
        Ok(PlacementCheck { id, name: row.name, state, pending, error })
    }

    /// Sets the `schema_state` of `id` to `state` if it is still `seen` (the other party has not changed it).
    async fn set_state(
        &self,
        id: PlacementId,
        seen: &str,
        state: SchemaState,
    ) -> Result<(), Error> {
        let at = now();
        placement::Entity::update_many()
            .col_expr(placement::Column::SchemaState, Expr::value(state.as_str()))
            .col_expr(placement::Column::SchemaCheckedAt, Expr::value(at))
            .filter(placement::Column::Id.eq(id.as_uuid()))
            .filter(placement::Column::SchemaState.eq(seen))
            .exec(self.registry.control().write())
            .await?;
        if seen != state.as_str() {
            self.registry.invalidate_placement(id);
            tracing::info!(placement_id = %id, from = seen, to = state.as_str(), "placement schema state changed");
        }
        Ok(())
    }

    /// Migration state of the control database (`main`) and then of each placement, by name.
    pub async fn status_all(&self) -> Result<Vec<PlacementMigrations>, Error> {
        let control = self.registry.control();
        let mut all = vec![PlacementMigrations {
            name: "main".to_owned(),
            migrations: self.migrator.status(control).await.map_err(migration_error)?,
        }];
        for row in self.rows().await? {
            let database = self.registry.placement_database(PlacementId::from_uuid(row.id)).await?;
            let migrations =
                self.migrator.status(&database).await.map_err(|error| {
                    Error::internal(format!("placement `{}`: {error}", row.name))
                })?;
            all.push(PlacementMigrations { name: row.name, migrations });
        }
        Ok(all)
    }

    /// Pending migrations of each placement (except `main`), or only of placement `only`; a dry run that
    /// changes nothing. A placement that does not exist: `placement_not_found`.
    pub async fn pending(&self, only: Option<&str>) -> Result<Vec<PlacementMigrations>, Error> {
        let mut all = Vec::new();
        for row in self.selected(only).await? {
            let database = self.registry.placement_database(PlacementId::from_uuid(row.id)).await?;
            let migrations =
                self.migrator.status(&database).await.map_err(|error| {
                    Error::internal(format!("placement `{}`: {error}", row.name))
                })?;
            let migrations = migrations.into_iter().filter(|state| !state.applied).collect();
            all.push(PlacementMigrations { name: row.name, migrations });
        }
        Ok(all)
    }

    /// Applies missing migrations at each placement (except `main`: `serve` does not run while `main` has
    /// pending migrations), or only at placement `only`; like the placement part of [`up_all`](Self::up_all): a
    /// failed placement does not stop the others, and the error names every failed placement. Takes the
    /// migration lock itself.
    pub async fn up_placements(
        &self,
        only: Option<&str>,
    ) -> Result<Vec<PlacementMigrations>, Error> {
        let mut lock = self.lock(LOCK_WAIT).await?;
        let mut report = MigrationReport { done: Vec::new(), failed: Vec::new() };
        let outcome = self.run_placements(only, &mut report, &mut lock).await;
        release(lock).await;
        outcome?;
        report.into_result()
    }

    /// Takes the lock of the placement migrations ([`MigrationLock::acquire_placements`]), waiting up to `wait`
    /// for another process. Every path that migrates placements holds it
    /// (D-20261126-placement-migration-exclusive): the command line, the API run, tenant moves. A registry
    /// without the database configuration (single mode, in-memory SQLite) has nothing to share it with.
    pub async fn lock(&self, wait: std::time::Duration) -> Result<MigrationLock, Error> {
        match self.registry.control_config() {
            Some(config) => MigrationLock::acquire_placements(config, wait)
                .await
                .map_err(|error| Error::internal(error.to_string())),
            None => Ok(MigrationLock::unshared()),
        }
    }

    /// Takes the main migration lock ([`MigrationLock::acquire`]) of the control database: what
    /// [`up_all_report`](Self::up_all_report) asks its caller to hold.
    pub async fn lock_main(&self, wait: std::time::Duration) -> Result<MigrationLock, Error> {
        match self.registry.control_config() {
            Some(config) => MigrationLock::acquire(config, wait)
                .await
                .map_err(|error| Error::internal(error.to_string())),
            None => Ok(MigrationLock::unshared()),
        }
    }

    /// Holding the lock: a placement still `migrating` was left so by a process that stopped midway (whoever
    /// migrates holds the lock); it becomes `behind` (its tenants keep getting 503 until migrated).
    pub(crate) async fn recover_migrating(&self) -> Result<(), Error> {
        for row in self.rows().await? {
            if row.schema_state == SchemaState::Migrating.as_str() {
                tracing::warn!(placement = %row.name, "placement left `migrating` by a stopped process");
                self.set_state(
                    PlacementId::from_uuid(row.id),
                    &row.schema_state,
                    SchemaState::Behind,
                )
                .await?;
            }
        }
        Ok(())
    }

    /// Holding the lock: the placement part of a migration run, recorded as one `kernel_migration_runs` row so
    /// the API sees it and does not queue a second run.
    async fn run_placements(
        &self,
        only: Option<&str>,
        report: &mut MigrationReport,
        lock: &mut MigrationLock,
    ) -> Result<(), Error> {
        let control = self.registry.control();
        self.recover_migrating().await?;
        let claim = super::migration_runs::claim::Claim::Direct { placement: only };
        let Some(run) = super::migration_runs::claim::claim(control, claim).await? else {
            return Ok(());
        };
        let outcome = async {
            for row in self.selected(only).await? {
                // Khoá Postgres mất ngầm khi kết nối đứt: dừng trước khi đụng placement kế tiếp.
                lock.verify().await.map_err(|error| Error::internal(error.to_string()))?;
                let name = row.name.clone();
                match self.try_placement(row).await? {
                    Ok(section) => report.done.push(section),
                    Err(error) => report.failed.push(PlacementFailure { name, error }),
                }
            }
            Ok::<(), Error>(())
        }
        .await;
        let applied = super::migration_runs::lists(
            report.done.iter().filter(|section| section.name != MAIN_NAME).cloned().collect(),
        );
        let mut errors: Vec<String> = report
            .failed
            .iter()
            .map(|failure| format!("placement `{}`: {}", failure.name, failure.error))
            .collect();
        if let Err(error) = &outcome {
            errors.push(error.to_string());
        }
        let error = (!errors.is_empty()).then(|| errors.join("\n"));
        super::migration_runs::claim::finish(control, run, Some(&applied), error).await?;
        outcome
    }

    async fn selected(&self, only: Option<&str>) -> Result<Vec<placement::Model>, Error> {
        let rows = self.rows().await?;
        match only {
            None => Ok(rows),
            Some(name) => {
                let found: Vec<placement::Model> =
                    rows.into_iter().filter(|row| row.name == name).collect();
                if found.is_empty() {
                    return Err(super::errors::tenant_placement_not_found(name));
                }
                Ok(found)
            }
        }
    }

    /// Migrates the placement `name` like `try_placement` (the API run, one placement at a time). A placement
    /// that does not exist: `placement_not_found`.
    pub(crate) async fn try_up_named(
        &self,
        name: &str,
    ) -> Result<Result<PlacementMigrations, String>, Error> {
        match self.selected(Some(name)).await?.into_iter().next() {
            Some(row) => self.try_placement(row).await,
            None => Err(super::errors::tenant_placement_not_found(name)),
        }
    }

    /// One placement: nothing pending → stays (or becomes) `ready` without going through `migrating` (its
    /// tenants never see a 503 for nothing); otherwise `migrating` → migrate + data upgrade → `ready`, on failure
    /// `behind`. A failure of the placement itself (cannot connect, a migration fails) is the inner `Err`, so
    /// the caller can go on with the next placement; the outer `Err` is a failure of the control database,
    /// which stops everything.
    async fn try_placement(
        &self,
        row: placement::Model,
    ) -> Result<Result<PlacementMigrations, String>, Error> {
        let id = PlacementId::from_uuid(row.id);
        let database = match self.registry.placement_database(id).await {
            Ok(database) => database,
            Err(error) => {
                self.set_state(id, &row.schema_state, SchemaState::Behind).await?;
                return Ok(Err(error.to_string()));
            }
        };
        match self.migrator.pending(&database).await {
            Ok(pending) if pending.is_empty() => {
                self.set_state(id, &row.schema_state, SchemaState::Ready).await?;
                return Ok(Ok(PlacementMigrations { name: row.name, migrations: Vec::new() }));
            }
            Ok(_) => {}
            Err(error) => {
                self.set_state(id, &row.schema_state, SchemaState::Behind).await?;
                return Ok(Err(error.to_string()));
            }
        }
        self.set_state(id, &row.schema_state, SchemaState::Migrating).await?;
        let migrating = SchemaState::Migrating.as_str();
        let applied = self.migrator.up(&database).await.map_err(|error| error.to_string());
        match applied {
            Ok(migrations) => {
                self.set_state(id, migrating, SchemaState::Ready).await?;
                Ok(Ok(PlacementMigrations { name: row.name, migrations }))
            }
            Err(error) => {
                self.set_state(id, migrating, SchemaState::Behind).await?;
                tracing::warn!(placement = %row.name, error = %error, "placement migrations failed");
                Ok(Err(error))
            }
        }
    }

    /// Applies missing migrations: the control database first, then every placement by name (sets
    /// `migrating` → migrate + data upgrade → `ready`). A placement that fails becomes `behind` and the others
    /// still run; the error names every failed placement. Running again continues from where it stopped (each
    /// database remembers its done part). [`up_all_report`](Self::up_all_report) returns what was applied
    /// alongside the failures.
    /// Takes the migration locks itself; a caller that holds the main one already uses `up_all_report`.
    pub async fn up_all(&self) -> Result<Vec<PlacementMigrations>, Error> {
        let main = self.lock_main(LOCK_WAIT).await?;
        let report = self.up_all_report(&main).await;
        release(main).await;
        report?.into_result()
    }

    /// Like [`up_all`](Self::up_all), but failed placements are part of the report instead of an error. `Err`
    /// only when the control database (`main`) cannot be migrated or read: then no placement is tried. `held`
    /// is the main migration lock ([`MigrationLock::acquire`]) the caller holds; the lock of the placements is
    /// taken here, after it (always main first, then placements).
    pub async fn up_all_report(&self, held: &MigrationLock) -> Result<MigrationReport, Error> {
        // `held` chỉ để buộc chỗ gọi giữ khoá chính (kiểu dữ liệu làm bằng chứng), không dùng tới.
        let _ = held;
        let control = self.registry.control();
        let mut report = MigrationReport {
            done: vec![PlacementMigrations {
                name: MAIN_NAME.to_owned(),
                migrations: self.migrator.up(control).await.map_err(migration_error)?,
            }],
            failed: Vec::new(),
        };
        let mut lock = self.lock(LOCK_WAIT).await?;
        let outcome = self.run_placements(None, &mut report, &mut lock).await;
        release(lock).await;
        outcome?;
        Ok(report)
    }
}

/// Result of [`Placements::up_all_report`]: the databases migrated (`main` first) and the placements that failed.
#[derive(Clone, Debug)]
pub struct MigrationReport {
    pub done: Vec<PlacementMigrations>,
    pub failed: Vec<PlacementFailure>,
}

/// A placement whose migrations failed; it is left `behind` (its tenants get `503 tenant_maintenance`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlacementFailure {
    pub name: String,
    pub error: String,
}

impl MigrationReport {
    /// `Ok` with the migrated databases when nothing failed; otherwise one error naming every failed placement.
    pub fn into_result(self) -> Result<Vec<PlacementMigrations>, Error> {
        if self.failed.is_empty() {
            return Ok(self.done);
        }
        let failures: Vec<String> = self
            .failed
            .iter()
            .map(|failure| format!("placement `{}`: {}", failure.name, failure.error))
            .collect();
        Err(Error::internal(format!("{}. {RETRY_HINT}", failures.join("; "))))
    }
}

/// What to do after a failed placement: shared by every error that names one.
const RETRY_HINT: &str = "Fix it and run the migrations again (`autoxide migrate up --all`, or \
                          `POST /api/v1/kernel/placements/migrations`): what is already applied is \
                          skipped";

impl Placements {
    /// Checks every placement now, then every `every` (spec: every minute) until `stop` completes: a
    /// placement just migrated on another machine becomes `ready`, a placement that lost its connection becomes
    /// `behind`. Errors are only logged: the next round retries.
    pub async fn watch(self, every: std::time::Duration, stop: impl Future<Output = ()>) {
        let mut stop = std::pin::pin!(stop);
        let mut ticker = tokio::time::interval(every);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = &mut stop => return,
                _ = ticker.tick() => {
                    if let Err(error) = self.check().await {
                        tracing::warn!(error = %error, "cannot check the placements");
                    }
                }
            }
        }
    }
}

/// Name of the control database in reports.
const MAIN_NAME: &str = "main";

/// How long `up_all` / `up_placements` wait for another process that holds the migration lock: a big
/// migration may take minutes; longer, the other process is likely stuck and the operator needs to know.
const LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// Releases the lock; a failure is only logged (the lock is also released when its connection or file closes).
pub(crate) async fn release(lock: MigrationLock) {
    if let Err(error) = lock.release().await {
        tracing::warn!(error = %error, "cannot release the migration lock");
    }
}

fn migration_error(error: crate::migration::MigrationError) -> Error {
    Error::internal(format!("main: {error}"))
}
