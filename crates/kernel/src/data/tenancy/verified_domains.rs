//! Snapshot of every verified tenant domain → its tenant, for CORS (`http::cors`, D-20261125-tenant-cors).
//!
//! - The domains table lives only in the control database, so one query reads them all; an origin that is
//!   nobody's domain is a map lookup, never a query.
//! - Re-read lazily, at most once per TTL (`AUTOXIDE_TENANCY_CACHE_SECONDS`), by one party at a time: the
//!   others wait and use what it read.
//! - [`VerifiedDomains::invalidate`] (a domain was verified, revoked, failed or moved on this server) drops
//!   the snapshot at once and bumps a generation number: a read that started before the change does not
//!   store its now-stale result.
//! - A read that takes longer than [`READ_TIMEOUT`] (a hung control database) counts as failed.
//! - A failed read is remembered for [`FAILURE_PAUSE`]: while the control database does not answer, waiting
//!   requests fail at once (CORS refused, fail closed) instead of each one querying again in turn.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Order, Query};
use uuid::Uuid;

use super::TenantId;
use crate::database::Database;
use crate::error::Error;

/// At most this many verified domains are held (oldest first); beyond it the newest get no CORS and an
/// error is logged.
pub(crate) const MAX_VERIFIED_DOMAINS: u64 = 100_000;

/// How long a failed read is remembered before the next request may try again.
pub(crate) const FAILURE_PAUSE: Duration = Duration::from_secs(1);

/// A read of the snapshot taking longer than this fails: requests from tenant pages do not queue up to the
/// pool's own timeout behind a hung control database.
pub(crate) const READ_TIMEOUT: Duration = Duration::from_secs(3);

type Snapshot = Arc<HashMap<String, TenantId>>;

#[derive(Default)]
pub(crate) struct VerifiedDomains {
    /// The snapshot and when it was read; `None` until read or after [`VerifiedDomains::invalidate`].
    snapshot: Mutex<Option<(Instant, Snapshot)>>,
    /// When the last read failed.
    failed_at: Mutex<Option<Instant>>,
    /// Bumped by every invalidation.
    generation: AtomicU64,
    /// Number of reads of the table (for tests: a burst of requests causes one read).
    reads: AtomicU64,
    /// Only one party reads at a time.
    reading: tokio::sync::Mutex<()>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl VerifiedDomains {
    /// The tenant that verified `domain` (already normalized), reading `control` when the snapshot is
    /// missing or older than `ttl`.
    pub(crate) async fn owner(
        &self,
        control: &Database,
        ttl: Duration,
        domain: &str,
    ) -> Result<Option<TenantId>, Error> {
        if let Some(map) = self.fresh(ttl) {
            return Ok(map.get(domain).copied());
        }
        self.check_failure()?;
        // Một bên đọc lại, các bên khác chờ rồi dùng bản vừa đọc (không dội database khi bản cũ hết hạn).
        let _reading = self.reading.lock().await;
        if let Some(map) = self.fresh(ttl) {
            return Ok(map.get(domain).copied());
        }
        // Bên vừa đọc trước mình lỗi: trả lỗi ngay, không xếp hàng hỏi lại database đang hỏng.
        self.check_failure()?;
        let generation = self.generation.load(Ordering::SeqCst);
        let outcome = tokio::time::timeout(READ_TIMEOUT, read(control))
            .await
            .unwrap_or_else(|_| Err(unavailable()));
        let map = match outcome {
            Ok(map) => Arc::new(map),
            Err(error) => {
                *lock(&self.failed_at) = Some(Instant::now());
                return Err(error);
            }
        };
        self.reads.fetch_add(1, Ordering::Relaxed);
        *lock(&self.failed_at) = None;
        self.store(generation, map.clone());
        Ok(map.get(domain).copied())
    }

    /// Keeps `map`, read while the generation was `generation`, unless an invalidation happened since: a read
    /// that started before a revocation / move must not bring the old owner back for a whole TTL.
    fn store(&self, generation: u64, map: Snapshot) {
        let mut snapshot = lock(&self.snapshot);
        if self.generation.load(Ordering::SeqCst) == generation {
            *snapshot = Some((Instant::now(), map));
        }
    }

    /// Drops the snapshot: the next request reads the table again.
    pub(crate) fn invalidate(&self) {
        // Tăng thế hệ trong cùng khoá với lúc lưu: lượt đọc cũ không thể lưu sau dòng này.
        let mut snapshot = lock(&self.snapshot);
        self.generation.fetch_add(1, Ordering::SeqCst);
        *snapshot = None;
    }

    /// Number of times the table was read.
    #[cfg(test)]
    pub(crate) fn reads(&self) -> u64 {
        self.reads.load(Ordering::Relaxed)
    }

    fn fresh(&self, ttl: Duration) -> Option<Snapshot> {
        let snapshot = lock(&self.snapshot);
        snapshot.as_ref().filter(|(at, _)| at.elapsed() < ttl).map(|(_, map)| map.clone())
    }

    fn check_failure(&self) -> Result<(), Error> {
        match *lock(&self.failed_at) {
            Some(at) if at.elapsed() < FAILURE_PAUSE => Err(unavailable()),
            _ => Ok(()),
        }
    }
}

fn unavailable() -> Error {
    Error::unavailable(
        "tenant_domains_unavailable",
        "The tenants' domains could not be read just now. Retry in a moment.",
    )
}

async fn read(control: &Database) -> Result<HashMap<String, TenantId>, Error> {
    // tenant-scope: ảnh chụp tên miền đã xác minh của mọi khách, để biết origin thuộc khách nào.
    let select = Query::select()
        .column(Alias::new("domain"))
        .column(Alias::new("tenant_id"))
        .from(Alias::new("kernel_tenant_domains"))
        .and_where(Expr::col(Alias::new("status")).eq("verified"))
        .order_by(Alias::new("id"), Order::Asc)
        .limit(MAX_VERIFIED_DOMAINS + 1)
        .to_owned();
    let rows = control.read().query_all(&select).await?;
    if rows.len() as u64 > MAX_VERIFIED_DOMAINS {
        tracing::error!(
            limit = MAX_VERIFIED_DOMAINS,
            "more verified tenant domains than the CORS snapshot holds: the newest get no CORS"
        );
    }
    let mut map = HashMap::with_capacity(rows.len());
    for row in rows.iter().take(MAX_VERIFIED_DOMAINS as usize) {
        let domain = row.try_get::<String>("", "domain")?;
        map.insert(domain, TenantId::from_uuid(row.try_get::<Uuid>("", "tenant_id")?));
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, ConfigReader, ConfigSource};
    use crate::migration::{MigrationTrait, Migrator, ModuleMigrations};
    use crate::tenancy::{DatabaseRegistry, TenancyConfig, TenancyMode};
    use crate::time::Timestamp;
    use autoxide_testkit::TestDatabase;

    async fn registry(test: &TestDatabase, mode: TenancyMode) -> DatabaseRegistry {
        let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
        let mut reader = ConfigReader::new(&source);
        let config = Config::read(&mut reader);
        let (config, _) = reader.finish(config).unwrap();
        let database = Database::connect(&config.database).await.unwrap();
        let migrations =
            vec![Box::new(crate::tenancy::schema::CreateTenancy) as Box<dyn MigrationTrait>];
        Migrator::new(vec![ModuleMigrations::new("kernel", migrations)])
            .unwrap()
            .up(&database)
            .await
            .unwrap();
        let tenancy = TenancyConfig { mode, ..TenancyConfig::default() };
        DatabaseRegistry::new(database, config.database, tenancy)
    }

    async fn add(registry: &DatabaseRegistry, domain: &str, tenant: Uuid, status: &str) {
        let now = Timestamp::now();
        let insert = Query::insert()
            .into_table(Alias::new("kernel_tenant_domains"))
            .columns(
                ["id", "tenant_id", "domain", "status", "verification_token"]
                    .into_iter()
                    .chain(["created_at", "updated_at"])
                    .map(Alias::new),
            )
            .values_panic([
                Uuid::now_v7().into(),
                tenant.into(),
                domain.into(),
                status.into(),
                "token".into(),
                now.into(),
                now.into(),
            ])
            .to_owned();
        registry.control().write().execute(&insert).await.unwrap();
    }

    #[tokio::test]
    async fn nhieu_request_cung_luc_chi_mot_luot_doc_origin_la_khong_truy_van() {
        TestDatabase::run(async |test| {
            let registry = registry(test, TenancyMode::Multi).await;
            let owner = Uuid::now_v7();
            add(&registry, "shop.test", owner, "verified").await;
            add(&registry, "wait.test", owner, "pending").await;

            let lookups = (0..32).map(|index| {
                let registry = registry.clone();
                tokio::spawn(async move {
                    let domain = if index % 2 == 0 { "shop.test" } else { "nobody.test" };
                    registry.verified_domain_owner(domain).await.unwrap()
                })
            });
            let found: Vec<_> = futures_util::future::join_all(lookups).await;
            assert_eq!(registry.verified_domain_reads(), 1);
            assert_eq!(found[0].as_ref().unwrap(), &Some(TenantId::from_uuid(owner)));
            assert_eq!(found[1].as_ref().unwrap(), &None);
            // Origin lạ: tra bản đồ, không đọc thêm.
            for index in 0..500 {
                let domain = format!("random-{index}.test");
                assert_eq!(registry.verified_domain_owner(&domain).await.unwrap(), None);
            }
            assert_eq!(registry.verified_domain_owner("wait.test").await.unwrap(), None);
            assert_eq!(registry.verified_domain_reads(), 1);
            // Sửa khách không ép đọc lại; tên miền đổi thì đọc lại đúng một lần.
            registry.invalidate(TenantId::from_uuid(owner));
            registry.verified_domain_owner("shop.test").await.unwrap();
            assert_eq!(registry.verified_domain_reads(), 1);
            registry.invalidate_domain("shop.test");
            registry.verified_domain_owner("shop.test").await.unwrap();
            assert_eq!(registry.verified_domain_reads(), 2);
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn che_do_mot_khach_khong_truy_van() {
        TestDatabase::run(async |test| {
            let registry = registry(test, TenancyMode::Single).await;
            add(&registry, "shop.test", Uuid::now_v7(), "verified").await;
            assert_eq!(registry.verified_domain_owner("shop.test").await.unwrap(), None);
            assert_eq!(registry.verified_domain_reads(), 0);
        })
        .await
        .unwrap();
    }

    #[test]
    fn luot_doc_cu_khong_ghi_de_sau_khi_thu_hoi() {
        let snapshot = VerifiedDomains::default();
        let ttl = Duration::from_secs(60);
        let map: Snapshot =
            Arc::new(HashMap::from([("shop.test".to_owned(), TenantId::from_uuid(Uuid::nil()))]));
        // Lượt đọc lấy thế hệ, thu hồi chạy trước khi nó lưu: không lưu bản cũ.
        let generation = snapshot.generation.load(Ordering::SeqCst);
        snapshot.invalidate();
        snapshot.store(generation, map.clone());
        assert!(snapshot.fresh(ttl).is_none());
        // Lượt đọc bắt đầu sau thu hồi: lưu bình thường.
        snapshot.store(snapshot.generation.load(Ordering::SeqCst), map);
        assert!(snapshot.fresh(ttl).is_some());
    }
}
