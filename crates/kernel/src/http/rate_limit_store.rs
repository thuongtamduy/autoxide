//! Per-API-key rate limit counting SHARED across all servers, through the `kernel_throttle` table (step 40
//! P6, D-375).
//!
//! Enabled with `AUTOXIDE_HTTP_RATE_LIMIT_STORE=database`. To avoid adding a write for every request,
//! counting is **in batches**: each process takes a batch of `k = max(1, burst / 4)` units for a key with
//! [`Throttle::take`] (one SQL statement), spends them down in memory, and only asks again when they run out.
//! A fixed 60-second window, a limit of `max(per_minute, burst)` units per window; maximum error
//! `k × number of servers` per window (units taken to one server that it has not used up).
//!
//! If the database does not answer, the request is let through (logged, `http_rate_limit_store_errors_total`
//! counted): the limiter must not make a database incident worse; the request still fails at a later step if
//! the database is truly down.

use crate::time::Timestamp;
use jiff::SignedDuration;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use uuid::Uuid;

use super::rate_limit::Decision;
use crate::config::RateLimit;
use crate::tenancy::{DatabaseRegistry, TenantId};
use crate::throttle::Throttle;

/// The counting window.
const WINDOW: Duration = Duration::from_secs(60);
/// Purges old rows of the count table at most this often, once per process.
const PURGE_EVERY: Duration = Duration::from_secs(3600);
/// Prunes expired batches from memory every this many checks.
const PRUNE_EVERY: u64 = 1024;

/// A batch of units for one key in this process.
#[derive(Default)]
struct Lot {
    /// Units still usable.
    left: u32,
    /// Expires when the database's window ends.
    until: Option<Instant>,
    /// The database granted 0 units: blocked until `until` without asking again.
    denied: bool,
}

/// Batches per key (tenant, API key).
type Lots = HashMap<(TenantId, Uuid), Arc<tokio::sync::Mutex<Lot>>>;

pub(super) struct SharedCounter {
    throttle: Throttle,
    registry: DatabaseRegistry,
    lot: u32,
    // `Mutex` thường giữ bảng (đoạn ngắn, không `await`); mỗi key một `Mutex` của tokio, giữ qua lần
    // hỏi database — request cùng key chờ nhau thay vì cùng hỏi database.
    lots: Mutex<Lots>,
    checks: AtomicU64,
    last_purge: Mutex<Option<Instant>>,
}

impl SharedCounter {
    /// `None` when disabled (`per_minute = 0`).
    pub(super) fn new(limit: RateLimit, registry: DatabaseRegistry) -> Option<Self> {
        if limit.per_minute == 0 {
            return None;
        }
        let throttle =
            Throttle::new(crate::builtin::NAME, limit.per_minute.max(limit.burst), WINDOW).ok()?;
        Some(SharedCounter {
            throttle,
            registry,
            lot: (limit.burst / 4).max(1),
            lots: Mutex::new(HashMap::new()),
            checks: AtomicU64::new(0),
            last_purge: Mutex::new(None),
        })
    }

    /// Checks one request of API key `key` belonging to tenant `tenant`.
    pub(super) async fn check(&self, tenant: TenantId, key: Uuid) -> Decision {
        let now = Instant::now();
        let cell = {
            let Ok(mut lots) = self.lots.lock() else { return Decision::Allow };
            if self.checks.fetch_add(1, Ordering::Relaxed).is_multiple_of(PRUNE_EVERY) {
                // Lô của cửa sổ đã qua không còn dùng được: bỏ cho bảng không phình. Lô đang được
                // dùng (khoá bận) giữ lại.
                lots.retain(|_, lot| match lot.try_lock() {
                    Ok(lot) => lot.until.is_some_and(|until| until > now),
                    Err(_) => true,
                });
            }
            lots.entry((tenant, key)).or_default().clone()
        };
        let mut lot = cell.lock().await;
        let now = Instant::now();
        if let Some(until) = lot.until.filter(|until| *until > now) {
            if lot.left > 0 {
                lot.left -= 1;
                return Decision::Allow;
            }
            if lot.denied {
                return Decision::Deny(until - now);
            }
        }
        self.purge_now_and_then(now);
        // Khoá đếm có khách (bước 33): khách A dùng hết lượt không chặn khách B.
        let name = format!("api_key:{tenant}:{key}");
        let control = self.registry.default_tenant();
        match self.throttle.take(&control, &name, self.lot).await {
            Ok(taken) => {
                metrics::counter!("http_rate_limit_store_takes_total").increment(1);
                let until = now + taken.window_left;
                lot.until = Some(until);
                if taken.granted == 0 {
                    lot.left = 0;
                    lot.denied = true;
                    Decision::Deny(taken.window_left.max(Duration::from_millis(1)))
                } else {
                    lot.left = taken.granted - 1;
                    lot.denied = false;
                    Decision::Allow
                }
            }
            Err(error) => {
                metrics::counter!("http_rate_limit_store_errors_total").increment(1);
                tracing::warn!(
                    error = %error,
                    "counting the API key rate limit in the database failed; letting the request through"
                );
                Decision::Allow
            }
        }
    }

    /// Purges old count rows (all tenants) at most once an hour, in the background - no scheduled job needed
    /// (like the request log, D-370).
    fn purge_now_and_then(&self, now: Instant) {
        let Ok(mut last) = self.last_purge.lock() else { return };
        if last.is_some_and(|last| now.duration_since(last) < PURGE_EVERY) {
            return;
        }
        *last = Some(now);
        let throttle = self.throttle.clone();
        let database = self.registry.default_tenant().database().clone();
        tokio::spawn(async move {
            let before = Timestamp::now() - SignedDuration::from_mins(10);
            if let Err(error) = throttle.purge(&database, before).await {
                tracing::warn!(error = %error, "purging old API key rate limit counters failed");
            }
        });
    }
}
