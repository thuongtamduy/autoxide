//! Placement — nơi giữ dữ liệu của khách (đặc tả 33 "Sổ kết nối, job, event, migration trên nhiều
//! database", D-261): thêm (URL kết nối lưu mã hoá), liệt kê, kiểm migration (`schema_state`),
//! migrate mọi placement.
//!
//! - Mọi database có cùng schema (D-253 mục 2): placement chạy đúng bộ migration của database điều
//!   khiển. Upgrade dữ liệu trên placement dùng khoá gốc của database điều khiển
//!   ([`Migrator::keyed_on`]) — module mã hoá secret của mọi khách bằng khoá đó.
//! - `schema_state`: `ready` (đủ migration), `behind` (còn migration chưa áp dụng — khách trên đó
//!   nhận `tenant_maintenance`, job của nó đợi), `migrating` (`migrate up --all` đang chạy). Kiểm
//!   lại không đè `migrating`; mọi lệnh đổi trạng thái chỉ ghi khi trạng thái chưa bị bên khác
//!   đổi (so với giá trị vừa đọc).
//! - Placement `main` (database điều khiển) không có trạng thái riêng: `serve` đòi nó đủ migration
//!   trước khi khởi động.

use std::sync::Arc;

use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, Set,
};
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

/// Trạng thái schema của một placement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchemaState {
    /// Đủ migration.
    Ready,
    /// Còn migration chưa áp dụng (hay không kết nối được): khách trên đó nhận `503`.
    Behind,
    /// `autoxide migrate up --all` đang chạy trên nó.
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

/// Một placement mới. URL có mật khẩu: chỉ lưu mã hoá, không bao giờ in ra (`Debug` của
/// [`DatabaseUrl`] che mật khẩu).
#[derive(Clone, Debug)]
pub struct NewPlacement {
    name: String,
    kind: PlacementKind,
    url: DatabaseUrl,
    read_url: Option<DatabaseUrl>,
}

impl NewPlacement {
    /// Placement tên `name` (chữ thường, số, `-`; tối đa 63 ký tự), loại `kind`, ở `url`.
    pub fn new(name: impl Into<String>, kind: PlacementKind, url: DatabaseUrl) -> Self {
        NewPlacement { name: name.into(), kind, url, read_url: None }
    }

    /// Read replica (Postgres) cho lệnh đọc.
    #[must_use]
    pub fn read_url(mut self, url: DatabaseUrl) -> Self {
        self.read_url = Some(url);
        self
    }
}

/// Kết quả kiểm migration của một placement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlacementCheck {
    pub id: PlacementId,
    pub name: String,
    pub state: SchemaState,
    /// Số migration chưa áp dụng.
    pub pending: usize,
    /// Không kết nối hay không đọc được trạng thái migration (khi đó `state` là `behind`).
    pub error: Option<String>,
}

/// Migration của một database: `main` (database điều khiển) hay một placement.
#[derive(Clone, Debug)]
pub struct PlacementMigrations {
    pub name: String,
    pub migrations: Vec<MigrationState>,
}

/// Quản lý placement (lệnh `autoxide placement`, `autoxide migrate --all`, kiểm định kỳ của
/// `serve`). Clone rẻ.
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

fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

impl Placements {
    /// `migrator`: migration của mọi module (`Modules::migrator`).
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

    /// Mọi placement (kể cả `main`), theo tên. Không bao giờ có URL.
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

    /// Đặt (`Some`) hay bỏ (`None`, file của khách về nơi lưu chung) nơi lưu file riêng của
    /// placement `name`. Lưu mã hoá (mục đích [`PLACEMENT_STORAGE`]); không bao giờ trả lại khoá bí
    /// mật. Placement đang có khách thì từ chối (`tenant_placement_storage_in_use`: file của họ nằm
    /// ở nơi lưu cũ) trừ khi `force` — khi nơi lưu mới đã có đúng các file đó (vd đổi khoá truy cập).
    /// Không thử kết nối: lỗi cấu hình hiện ra ở lần đọc / ghi file đầu tiên (`autoxide doctor`).
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

    /// Thêm placement: kiểm tên, thử kết nối, lưu URL mã hoá, rồi kiểm migration (database đã
    /// migrate sẵn thì `ready` ngay; database trống thì `behind` tới khi `migrate up --all`).
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

    /// Kiểm migration của mọi placement (trừ `main`), cập nhật `schema_state`. Không kết nối được
    /// một placement không làm hỏng cả lệnh: placement đó thành `behind`, kèm lỗi.
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

    /// Đặt `schema_state` của `id` thành `state` nếu nó vẫn là `seen` (bên khác chưa đổi).
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

    /// Trạng thái migration của database điều khiển (`main`) rồi của từng placement, theo tên.
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

    /// Migration còn chờ của từng placement (trừ `main`), hay chỉ của placement `only` — chạy thử,
    /// không đổi gì. Placement không có: `placement_not_found`.
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

    /// Áp dụng migration còn thiếu ở từng placement (trừ `main`: `serve` không chạy khi `main` còn
    /// migration chờ), hay chỉ ở placement `only` — như phần placement của [`up_all`](Self::up_all).
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

    /// Một placement: `migrating` → migrate + upgrade dữ liệu → `ready`; hỏng thì `behind`.
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

    /// Áp dụng migration còn thiếu: database điều khiển trước, rồi từng placement theo tên (đặt
    /// `migrating` → migrate + upgrade dữ liệu → `ready`). Hỏng ở một placement thì dừng, đặt nó
    /// `behind` và báo tên; chạy lại thì tiếp từ chỗ dừng (mỗi database tự nhớ phần đã xong).
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
    /// Kiểm mọi placement ngay, rồi mỗi `every` (đặc tả: mỗi phút) cho tới khi `stop` xong —
    /// placement vừa được migrate ở máy khác thành `ready`, placement mất kết nối thành `behind`.
    /// Lỗi chỉ ghi log: lần sau thử lại.
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
