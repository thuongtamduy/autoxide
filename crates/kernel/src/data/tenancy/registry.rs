//! Sổ database (đặc tả 33 "Sổ kết nối"): database điều khiển (`DATABASE_URL`, placement `main`) và
//! các placement khác, mở lười, tối đa `AUTOXIDE_TENANCY_MAX_OPEN_DATABASES` pool cùng lúc.
//!
//! - Chế độ `single`: mọi thứ là khách mặc định trên database điều khiển; không truy vấn gì thêm —
//!   đường nóng như trước bước 33.
//! - Chế độ `multi`: tra khách theo ID / slug / tên miền qua bảng mặt điều khiển, kết quả giữ trong
//!   cache có TTL (`AUTOXIDE_TENANCY_CACHE_SECONDS`); sửa khách trên máy chủ này thì
//!   [`DatabaseRegistry::invalidate`] ngay, máy chủ khác thấy sau tối đa TTL.
//! - URL kết nối của placement lưu mã hoá (`Sealer`, mục đích [`PLACEMENT_URL`], gắn với ID
//!   placement); sổ mở bằng khoá gốc của chương trình — module `kernel` đưa khoá vào lúc khởi
//!   động ([`DatabaseRegistry::use_secret_key`]).
//! - Khách trên placement có `schema_state` khác `ready` mang `schema_ready = false` → nhận
//!   `tenant_maintenance` (đặc tả 33); khách nơi khác chạy bình thường.
//! - Pool ít dùng nhất bị bỏ khỏi sổ khi cần chỗ; request đang giữ bản clone của nó vẫn chạy tới
//!   xong (pool tự đóng khi không còn bản clone nào) — không bao giờ gọi `close()` ở đây.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Query, SelectStatement};
use sea_orm::{ConnectionTrait, QueryResult};
use uuid::Uuid;

use super::{
    DEFAULT_TENANT, MAIN_PLACEMENT, NO_TENANT, PlacementId, TenancyConfig, TenancyMode,
    TenantContext, TenantId, TenantStatus, errors,
};
use crate::config::{DatabaseConfig, DatabaseUrl};
use crate::database::Database;
use crate::error::Error;
use crate::secret::SecretKey;

/// Mục đích (AAD của `Sealer`) của URL kết nối placement; dòng là ID placement.
pub const PLACEMENT_URL: &str = "kernel.placement_url";
/// Mục đích của URL đọc (read replica) của placement.
pub const PLACEMENT_READ_URL: &str = "kernel.placement_read_url";

/// Sổ database của bản cài. Clone rẻ (bên trong là `Arc`).
#[derive(Clone)]
pub struct DatabaseRegistry(Arc<Inner>);

struct Inner {
    mode: TenancyMode,
    config: TenancyConfig,
    control: Database,
    /// Cấu hình của database điều khiển: mẫu khi mở placement khác (thời gian chờ…).
    base: Option<DatabaseConfig>,
    default: TenantContext,
    tenants: Mutex<HashMap<TenantId, (Instant, TenantContext)>>,
    slugs: Mutex<HashMap<String, (Instant, TenantId)>>,
    domains: Mutex<HashMap<String, (Instant, Option<TenantId>)>>,
    pools: Mutex<Pools>,
    /// Khoá gốc để mở URL placement đã mã hoá; module `kernel` đặt lúc khởi động.
    secret: OnceLock<SecretKey>,
    /// Chỉ một bên mở pool của placement cùng lúc (hai request cùng cần một placement chưa mở
    /// thì bên sau dùng pool bên trước vừa mở).
    opening: tokio::sync::Mutex<()>,
}

#[derive(Default)]
struct Pools {
    open: HashMap<PlacementId, (u64, Database)>,
    tick: u64,
}

impl std::fmt::Debug for DatabaseRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DatabaseRegistry").field("mode", &self.0.mode).finish_non_exhaustive()
    }
}

/// Khoá `mutex` dù một thread khác đã panic lúc giữ nó: dữ liệu bên trong chỉ là cache.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl DatabaseRegistry {
    /// Sổ của chế độ `single`: mọi request là khách mặc định trên `control`.
    pub fn single(control: Database) -> Self {
        Self::build(control, None, TenancyConfig::default())
    }

    /// Sổ theo `config`: `base` là cấu hình của database điều khiển (`DATABASE_URL`), dùng làm mẫu
    /// khi mở placement khác.
    pub fn new(control: Database, base: DatabaseConfig, config: TenancyConfig) -> Self {
        Self::build(control, Some(base), config)
    }

    fn build(control: Database, base: Option<DatabaseConfig>, config: TenancyConfig) -> Self {
        DatabaseRegistry(Arc::new(Inner {
            mode: config.mode,
            default: TenantContext::default_on(control.clone()),
            config,
            control,
            base,
            tenants: Mutex::default(),
            slugs: Mutex::default(),
            domains: Mutex::default(),
            pools: Mutex::default(),
            secret: OnceLock::new(),
            opening: tokio::sync::Mutex::new(()),
        }))
    }

    pub fn mode(&self) -> TenancyMode {
        self.0.mode
    }

    pub fn config(&self) -> &TenancyConfig {
        &self.0.config
    }

    /// Database điều khiển (placement `main`): bảng mặt điều khiển và dữ liệu của khách mặc định.
    pub fn control(&self) -> &Database {
        &self.0.control
    }

    /// Khách mặc định (khách nền tảng) trên database điều khiển.
    pub fn default_tenant(&self) -> TenantContext {
        self.0.default.clone()
    }

    fn ttl(&self) -> Duration {
        Duration::from_secs(self.0.config.cache_seconds)
    }

    /// Ngữ cảnh của khách `id`. Không có: `tenant_not_found`.
    pub async fn tenant(&self, id: TenantId) -> Result<TenantContext, Error> {
        if self.0.mode == TenancyMode::Single {
            return if id == DEFAULT_TENANT {
                Ok(self.default_tenant())
            } else {
                Err(errors::tenant_not_found(&id.to_string()))
            };
        }
        // Đọc cache ở một câu riêng: khoá được trả ngay, không giữ qua `await` bên dưới.
        let cached = lock(&self.0.tenants).get(&id).cloned();
        if let Some((at, tenant)) = cached
            && at.elapsed() < self.ttl()
        {
            return Ok(tenant);
        }
        let select = tenant_select().and_where(tenant_column("id").eq(id.as_uuid())).to_owned();
        let row = self.0.control.read().query_one(&select).await?;
        let row = row.ok_or_else(|| errors::tenant_not_found(&id.to_string()))?;
        self.context_of(&row).await
    }

    /// Ngữ cảnh của khách có tên rút gọn `slug`.
    pub async fn tenant_by_slug(&self, slug: &str) -> Result<TenantContext, Error> {
        if self.0.mode == TenancyMode::Single {
            return if slug == self.0.default.slug() {
                Ok(self.default_tenant())
            } else {
                Err(errors::tenant_not_found(slug))
            };
        }
        let cached = lock(&self.0.slugs).get(slug).cloned();
        if let Some((at, id)) = cached
            && at.elapsed() < self.ttl()
        {
            return self.tenant(id).await;
        }
        let select = tenant_select().and_where(tenant_column("slug").eq(slug)).to_owned();
        let row = self.0.control.read().query_one(&select).await?;
        let row = row.ok_or_else(|| errors::tenant_not_found(slug))?;
        self.context_of(&row).await
    }

    /// Khách sở hữu tên miền `host` (đã xác minh), nếu có. Chế độ `single`: luôn `None` (không đọc
    /// tên miền).
    pub async fn tenant_by_domain(&self, host: &str) -> Result<Option<TenantContext>, Error> {
        if self.0.mode == TenancyMode::Single {
            return Ok(None);
        }
        let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
        let cached = lock(&self.0.domains).get(&host).cloned();
        if let Some((at, id)) = cached
            && at.elapsed() < self.ttl()
        {
            return match id {
                Some(id) => self.tenant(id).await.map(Some),
                None => Ok(None),
            };
        }
        let select = Query::select()
            .column(Alias::new("tenant_id"))
            .from(Alias::new("kernel_tenant_domains"))
            .and_where(Expr::col(Alias::new("domain")).eq(host.as_str()))
            .and_where(Expr::col(Alias::new("status")).eq("verified"))
            .to_owned();
        let found = self.0.control.read().query_one(&select).await?;
        let id = match found {
            Some(row) => Some(TenantId::from_uuid(row.try_get::<Uuid>("", "tenant_id")?)),
            None => None,
        };
        lock(&self.0.domains).insert(host, (Instant::now(), id));
        match id {
            Some(id) => self.tenant(id).await.map(Some),
            None => Ok(None),
        }
    }

    /// Khoá gốc dùng mở URL placement đã mã hoá. Đặt một lần (module `kernel` đặt lúc khởi động);
    /// lần sau bị bỏ qua.
    pub fn use_secret_key(&self, key: SecretKey) {
        let _ = self.0.secret.set(key);
    }

    /// Bỏ mọi khách trên placement `id` khỏi cache (sau khi `schema_state` của nó đổi).
    pub fn invalidate_placement(&self, id: PlacementId) {
        let gone: Vec<TenantId> = lock(&self.0.tenants)
            .iter()
            .filter(|(_, (_, tenant))| tenant.placement() == id)
            .map(|(tenant, _)| *tenant)
            .collect();
        for tenant in gone {
            self.invalidate(tenant);
        }
    }

    /// Bỏ tên miền `domain` khỏi cache (sau khi xác minh, thu hồi hay đổi khách của nó trên máy chủ
    /// này): request kế tiếp tra lại bảng.
    pub fn invalidate_domain(&self, domain: &str) {
        let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
        lock(&self.0.domains).remove(&domain);
    }

    /// Bỏ khách `id` khỏi cache (sau khi sửa khách trên máy chủ này).
    pub fn invalidate(&self, id: TenantId) {
        lock(&self.0.tenants).remove(&id);
        lock(&self.0.slugs).retain(|_, (_, known)| *known != id);
        lock(&self.0.domains).retain(|_, (_, known)| *known != Some(id));
    }

    /// Mọi database máy chủ này chạy job và event cho: `(placement, tên, database)` — chỉ placement
    /// đã đủ migration. Chế độ `single`: chỉ database điều khiển.
    pub async fn databases(&self) -> Result<Vec<(PlacementId, String, Database)>, Error> {
        let mut all = Vec::new();
        if self.0.config.worker_placements.includes("main") {
            all.push((MAIN_PLACEMENT, "main".to_owned(), self.0.control.clone()));
        }
        if self.0.mode == TenancyMode::Single {
            return Ok(all);
        }
        // Placement chưa đủ migration: job và event của nó đợi tới khi `migrate up --all` xong
        // (khách trên đó cũng đang nhận `tenant_maintenance`).
        let select = Query::select()
            .columns(["id", "name"].map(Alias::new))
            .from(Alias::new("kernel_placements"))
            .and_where(Expr::col(Alias::new("id")).ne(MAIN_PLACEMENT.as_uuid()))
            .and_where(Expr::col(Alias::new("schema_state")).eq("ready"))
            .order_by(Alias::new("name"), sea_orm::sea_query::Order::Asc)
            .to_owned();
        for row in self.0.control.read().query_all(&select).await? {
            let name: String = row.try_get("", "name")?;
            if !self.0.config.worker_placements.includes(&name) {
                continue;
            }
            let id = PlacementId::from_uuid(row.try_get::<Uuid>("", "id")?);
            all.push((id, name, self.placement_database(id).await?));
        }
        Ok(all)
    }

    /// Database của placement `id`; mở pool nếu chưa mở.
    pub async fn placement_database(&self, id: PlacementId) -> Result<Database, Error> {
        if id == MAIN_PLACEMENT {
            return Ok(self.0.control.clone());
        }
        if let Some(database) = self.touch(id) {
            return Ok(database);
        }
        let _opening = self.0.opening.lock().await;
        if let Some(database) = self.touch(id) {
            return Ok(database);
        }
        let select = Query::select()
            .columns(["url_sealed", "read_url_sealed"].map(Alias::new))
            .from(Alias::new("kernel_placements"))
            .and_where(Expr::col(Alias::new("id")).eq(id.as_uuid()))
            .to_owned();
        let row = self.0.control.read().query_one(&select).await?;
        let row = row.ok_or_else(|| errors::tenant_placement_not_found(&id.to_string()))?;
        let url: Option<String> = row.try_get("", "url_sealed")?;
        let read_url: Option<String> = row.try_get("", "read_url_sealed")?;
        let Some(url) = url else {
            // Không có URL riêng: cùng database điều khiển.
            return Ok(self.0.control.clone());
        };
        let url = self.unseal(id, PLACEMENT_URL, &url).await?;
        let read_url = match read_url {
            Some(sealed) => Some(self.unseal(id, PLACEMENT_READ_URL, &sealed).await?),
            None => None,
        };
        let database = self.open(&url, read_url.as_deref()).await?;
        let mut pools = lock(&self.0.pools);
        pools.tick += 1;
        let tick = pools.tick;
        pools.open.insert(id, (tick, database.clone()));
        while pools.open.len() > self.0.config.max_open_databases {
            // Bỏ pool ít dùng nhất khỏi sổ; bản clone đang dùng vẫn chạy tới xong.
            let oldest = pools.open.iter().min_by_key(|(_, (used, _))| *used).map(|(id, _)| *id);
            match oldest {
                Some(oldest) => {
                    pools.open.remove(&oldest);
                }
                None => break,
            }
        }
        Ok(database)
    }

    /// Số pool placement đang mở (không tính database điều khiển).
    pub fn open_databases(&self) -> usize {
        lock(&self.0.pools).open.len()
    }

    fn touch(&self, id: PlacementId) -> Option<Database> {
        let mut pools = lock(&self.0.pools);
        pools.tick += 1;
        let tick = pools.tick;
        pools.open.get_mut(&id).map(|(used, database)| {
            *used = tick;
            database.clone()
        })
    }

    /// Mở URL placement đã mã hoá. Giá trị chưa mã hoá (chép tay vào bảng) trả nguyên văn.
    async fn unseal(&self, id: PlacementId, purpose: &str, sealed: &str) -> Result<String, Error> {
        if !crate::secret::is_sealed(sealed) {
            return Ok(sealed.to_owned());
        }
        let plain = self.sealer().await?.open(NO_TENANT, purpose, &id.to_string(), sealed)?;
        String::from_utf8(plain.to_vec())
            .map_err(|_| Error::internal(format!("placement {id}: the stored URL is not text")))
    }

    /// Bộ mã hoá của khoá gốc (mã hoá / mở URL placement).
    pub(super) async fn sealer(&self) -> Result<crate::secret::Sealer, Error> {
        let key = self.0.secret.get().ok_or_else(|| {
            Error::internal("the database registry has no secret key to open placement URLs")
        })?;
        key.sealer().await
    }

    /// Mở pool cho một placement (cỡ pool nhỏ của `AUTOXIDE_TENANCY_*_CONNECTIONS`).
    pub(super) async fn open(&self, url: &str, read_url: Option<&str>) -> Result<Database, Error> {
        let parse = |raw: &str| {
            raw.parse::<DatabaseUrl>()
                .map_err(|error| Error::internal(format!("placement URL is invalid: {error}")))
        };
        let wait = self.0.base.as_ref().map_or(Duration::from_secs(5), |base| base.wait);
        let config = DatabaseConfig {
            url: parse(url)?,
            read_url: read_url.map(parse).transpose()?,
            write_connections: self.0.config.write_connections,
            read_connections: self.0.config.read_connections,
            wait,
        };
        Database::connect(&config).await.map_err(|error| Error::internal(error.to_string()))
    }

    async fn context_of(&self, row: &QueryResult) -> Result<TenantContext, Error> {
        let id = TenantId::from_uuid(row.try_get::<Uuid>("", "id")?);
        let slug: String = row.try_get("", "slug")?;
        let name: String = row.try_get("", "name")?;
        let status: String = row.try_get("", "status")?;
        let status = TenantStatus::parse(&status).ok_or_else(|| {
            Error::internal(format!("tenant {id} has an unknown status `{status}`"))
        })?;
        let placement = PlacementId::from_uuid(row.try_get::<Uuid>("", "placement_id")?);
        let schema_state: Option<String> = row.try_get("", "schema_state")?;
        // Database điều khiển luôn đủ migration (`serve` đòi trước khi khởi động).
        let ready = placement == MAIN_PLACEMENT || schema_state.as_deref() == Some("ready");
        let database = self.placement_database(placement).await?;
        let tenant = TenantContext::new(id, slug.clone(), name, status, placement, database)
            .with_schema_ready(ready);
        let now = Instant::now();
        lock(&self.0.tenants).insert(id, (now, tenant.clone()));
        lock(&self.0.slugs).insert(slug, (now, id));
        Ok(tenant)
    }
}

/// Cột `column` của `kernel_tenants` (ghi rõ bảng: câu truy vấn nối thêm `kernel_placements`).
fn tenant_column(column: &'static str) -> Expr {
    Expr::col((Alias::new("kernel_tenants"), Alias::new(column)))
}

/// Khách kèm `schema_state` của placement chứa nó.
fn tenant_select() -> SelectStatement {
    let tenants = Alias::new("kernel_tenants");
    let placements = Alias::new("kernel_placements");
    Query::select()
        .columns(
            ["id", "slug", "name", "status", "placement_id"]
                .map(|column| (tenants.clone(), Alias::new(column))),
        )
        .column((placements.clone(), Alias::new("schema_state")))
        .from(tenants.clone())
        .left_join(
            placements.clone(),
            Expr::col((placements, Alias::new("id"))).equals((tenants, Alias::new("placement_id"))),
        )
        .to_owned()
}
