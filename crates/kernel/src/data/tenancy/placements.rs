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
use crate::migration::{MigrationState, Migrator};
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
    /// pending migrations), or only at placement `only`; like the placement part of [`up_all`](Self::up_all).
    pub async fn up_placements(
        &self,
        only: Option<&str>,
    ) -> Result<Vec<PlacementMigrations>, Error> {
        let mut all = Vec::new();
        for row in self.selected(only).await? {
            all.push(self.up_placement(row).await?);
        }
        Ok(all)
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

    /// One placement: `migrating` → migrate + data upgrade → `ready`; on failure `behind`.
    async fn up_placement(&self, row: placement::Model) -> Result<PlacementMigrations, Error> {
        let id = PlacementId::from_uuid(row.id);
        self.set_state(id, &row.schema_state, SchemaState::Migrating).await?;
        let migrating = SchemaState::Migrating.as_str();
        let applied = match self.registry.placement_database(id).await {
            Ok(database) => self.migrator.up(&database).await.map_err(|error| error.to_string()),
            Err(error) => Err(error.to_string()),
        };
        match applied {
            Ok(migrations) => {
                self.set_state(id, migrating, SchemaState::Ready).await?;
                Ok(PlacementMigrations { name: row.name, migrations })
            }
            Err(error) => {
                self.set_state(id, migrating, SchemaState::Behind).await?;
                Err(Error::internal(format!(
                    "placement `{}`: {error}. Fix it and run the migrations again (`autoxide \
                     migrate up --all`, or `POST /api/v1/kernel/placements/migrations`): what \
                     is already applied is skipped",
                    row.name
                )))
            }
        }
    }

    /// Applies missing migrations: the control database first, then each placement by name (sets
    /// `migrating` → migrate + data upgrade → `ready`). If one placement fails, stops, sets it `behind` and
    /// reports its name; running again continues from where it stopped (each database remembers its done part).
    pub async fn up_all(&self) -> Result<Vec<PlacementMigrations>, Error> {
        let control = self.registry.control();
        let mut all = vec![PlacementMigrations {
            name: "main".to_owned(),
            migrations: self.migrator.up(control).await.map_err(migration_error)?,
        }];
        for row in self.rows().await? {
            all.push(self.up_placement(row).await?);
        }
        Ok(all)
    }
}

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

fn migration_error(error: crate::migration::MigrationError) -> Error {
    Error::internal(format!("main: {error}"))
}
