//! Rate limit per tenant (D-20261126-tenant-rate-limit): the third layer after the limit per client address and
//! the limit per API key, so one busy tenant cannot crowd out the others.
//!
//! - Level: the tenant's plan, limit `kernel.api_rate_per_minute` (requests per minute; the burst equals the
//!   rate), when the plan sets it — "unlimited" included; otherwise `AUTOXIDE_HTTP_RATE_LIMIT_TENANT_PER_MINUTE`
//!   / `_BURST` (`0`: off). The plan comes from the entitlements snapshot, which `plans` caches: no query per
//!   request.
//! - Counted like the limit per API key: in each process's memory (GCRA), or shared by every server through the
//!   database with `AUTOXIDE_HTTP_RATE_LIMIT_STORE=database` (fixed one-minute windows taken in lots).
//! - Applied to every module's REST routes once the request's tenant is resolved (the same cached resolution
//!   the handlers use) and to every MCP tool call. A request whose tenant cannot be resolved (a public page with
//!   no tenant) only meets the limit per address.
//! - Over the limit: 429 `tenant_rate_limited` with `Retry-After`.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use uuid::Uuid;

use super::rate_limit::Decision;
use super::rate_limit_store::SharedCounter;
use crate::config::RateLimit;
use crate::entitlement::{EntitlementsHandle, Limit};
use crate::error::Error;
use crate::tenancy::{DatabaseRegistry, TenantId};

/// The plan's request rate of a tenant (declared by the kernel).
pub const API_RATE: Limit = Limit::amount(
    "kernel.api_rate_per_minute",
    "API requests (REST and MCP) a tenant may make per minute, all its users and API keys together; \
     more are answered with 429 `tenant_rate_limited`. A plan that does not set it keeps the server's \
     AUTOXIDE_HTTP_RATE_LIMIT_TENANT_PER_MINUTE.",
    "requests per minute",
);

/// Forgets tenants that have gone quiet every this many requests (memory store).
const PRUNE_EVERY: u64 = 1024;

pub(crate) struct TenantLimiter {
    default: RateLimit,
    entitlements: EntitlementsHandle,
    /// `Some`: counted through the database (shared by every server).
    store: Option<DatabaseRegistry>,
    // `Mutex` chuẩn: giữ trong vài phép tính, không qua `.await`.
    tats: Mutex<HashMap<TenantId, Instant>>,
    shared: Mutex<HashMap<(u32, u32), Arc<SharedCounter>>>,
    requests: AtomicU64,
}

impl TenantLimiter {
    pub(crate) fn new(
        default: RateLimit,
        entitlements: EntitlementsHandle,
        store: Option<DatabaseRegistry>,
    ) -> Self {
        TenantLimiter {
            default,
            entitlements,
            store,
            tats: Mutex::default(),
            shared: Mutex::default(),
            requests: AtomicU64::new(0),
        }
    }

    /// The level of tenant `tenant`; `None`: not limited.
    async fn level(&self, tenant: TenantId) -> Result<Option<RateLimit>, Error> {
        if self.entitlements.is_enforced() {
            let snapshot = self.entitlements.snapshot(tenant).await?;
            if !snapshot.unrestricted
                && let Some(planned) = snapshot.limits.get(API_RATE.key())
            {
                return Ok(planned.map(|rate| {
                    let rate = u32::try_from(rate.max(0)).unwrap_or(u32::MAX);
                    RateLimit { per_minute: rate, burst: rate.max(1) }
                }));
            }
        }
        Ok((self.default.per_minute > 0).then_some(self.default))
    }

    /// Counts one request of tenant `tenant`.
    pub(super) async fn check(&self, tenant: TenantId) -> Result<Decision, Error> {
        let Some(level) = self.level(tenant).await? else { return Ok(Decision::Allow) };
        if level.per_minute == 0 {
            // Gói đặt 0: không request nào qua.
            return Ok(Decision::Deny(Duration::from_secs(60)));
        }
        match &self.store {
            Some(registry) => {
                let counter = self.counter(level, registry);
                Ok(match counter {
                    Some(counter) => counter.check(tenant, Uuid::nil()).await,
                    None => Decision::Allow,
                })
            }
            None => Ok(self.check_memory(tenant, level, Instant::now())),
        }
    }

    /// The shared counter for level `level` (one per distinct level, so plans with different levels count apart).
    fn counter(&self, level: RateLimit, registry: &DatabaseRegistry) -> Option<Arc<SharedCounter>> {
        let mut shared = self.shared.lock().ok()?;
        let key = (level.per_minute, level.burst);
        if let Some(found) = shared.get(&key) {
            return Some(found.clone());
        }
        let counter = Arc::new(SharedCounter::named(level, registry.clone(), "tenant")?);
        shared.insert(key, counter.clone());
        Some(counter)
    }

    /// GCRA in memory with the tenant's own level (see `rate_limit`).
    fn check_memory(&self, tenant: TenantId, level: RateLimit, now: Instant) -> Decision {
        let Ok(mut tats) = self.tats.lock() else { return Decision::Allow };
        if self.requests.fetch_add(1, Ordering::Relaxed).is_multiple_of(PRUNE_EVERY) {
            tats.retain(|_, tat| *tat > now);
        }
        let interval = Duration::from_secs(60) / level.per_minute;
        let tolerance = interval * (level.burst.max(1) - 1);
        let tat = tats.get(&tenant).copied().unwrap_or(now).max(now);
        let ahead = tat - now;
        if ahead > tolerance {
            return Decision::Deny(ahead - tolerance);
        }
        tats.insert(tenant, tat + interval);
        Decision::Allow
    }
}

/// The 429 answer.
pub(crate) fn tenant_rate_limited(wait: Duration) -> Error {
    metrics::counter!("http_tenant_rate_limited_total").increment(1);
    let seconds = wait.as_secs() + u64::from(wait.subsec_nanos() > 0);
    Error::too_many_requests(
        "tenant_rate_limited",
        "Too many requests for this tenant. Wait {seconds} seconds, then retry; send requests more \
         slowly, or move to a plan with a higher rate.",
    )
    .with_param("seconds", seconds)
    .with_retry_after(seconds)
}

/// The HTTP layer of a module's routes.
pub(crate) async fn limit(limiter: Arc<TenantLimiter>, request: Request, next: Next) -> Response {
    let (mut parts, body) = request.into_parts();
    // Khách giải như handler (kết quả được giữ trong request). Không giải được: chỉ chịu tầng theo địa chỉ.
    if let Ok(tenant) = crate::tenancy::request::tenant(&mut parts).await {
        match limiter.check(tenant.id()).await {
            Ok(Decision::Allow) => {}
            Ok(Decision::Deny(wait)) => return tenant_rate_limited(wait).into_response(),
            Err(error) => return error.into_response(),
        }
    }
    next.run(Request::from_parts(parts, body)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moi_khach_dem_rieng_theo_muc_cua_minh() {
        let limiter = TenantLimiter::new(
            RateLimit { per_minute: 60, burst: 2 },
            EntitlementsHandle::default(),
            None,
        );
        let (a, b) =
            (TenantId::from_uuid(Uuid::from_u128(1)), TenantId::from_uuid(Uuid::from_u128(2)));
        let level = RateLimit { per_minute: 60, burst: 2 };
        let now = Instant::now();
        assert_eq!(limiter.check_memory(a, level, now), Decision::Allow);
        assert_eq!(limiter.check_memory(a, level, now), Decision::Allow);
        assert!(matches!(limiter.check_memory(a, level, now), Decision::Deny(_)));
        // Khách B không bị khách A làm nghẽn.
        assert_eq!(limiter.check_memory(b, level, now), Decision::Allow);
        assert_eq!(limiter.check_memory(a, level, now + Duration::from_secs(1)), Decision::Allow);
    }

    #[test]
    fn ma_loi_va_retry_after() {
        let error = tenant_rate_limited(Duration::from_millis(1500));
        assert_eq!((error.code(), error.retry_after()), ("tenant_rate_limited", Some(2)));
    }
}
