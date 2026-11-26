//! Rate limiting: limits the number of requests from each client address.
//!
//! The GCRA algorithm (Generic Cell Rate Algorithm - a "token bucket" style calculation that needs to
//! remember only ONE timestamp per client): each request pushes the "theoretical arrival time of the next
//! request" (`tat`) forward by an `interval` (= 1 minute / requests per minute). A request is rejected when
//! that timestamp is more than `tolerance` (= `interval` × (burst − 1)) ahead of the present - meaning the
//! client has used up its burst allowance and is sending faster than the permitted rate.
//!
//! Counting is in each process's memory: with several servers, each server counts separately. That is
//! enough to stop one client from bursting; stopping password guessing is the job of login throttling
//! (counted in the database, shared by all servers). The per-API-key limit can be counted jointly through
//! the database when `AUTOXIDE_HTTP_RATE_LIMIT_STORE=database` (step 40 P6, `rate_limit_store`).

use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use uuid::Uuid;

use super::client::ClientIp;
use crate::access::{Authenticator, Principal};
use crate::config::RateLimit;
use crate::error::Error;

/// Cleans up clients that have gone "cold" after this many requests.
const PRUNE_EVERY: u64 = 1024;

pub(super) struct RateLimiter {
    cells: Gcra<IpAddr>,
}

/// The GCRA counter for any kind of key (`K`: client address, API key ID...).
//
// `K: Eq + Hash + Copy`: khoá làm chìa của `HashMap` và chép được rẻ (như `IpAddr`, `Uuid`).
struct Gcra<K> {
    interval: Duration,
    tolerance: Duration,
    // `Mutex` của thư viện chuẩn (không phải của tokio): đoạn giữ khoá rất ngắn và
    // không có `await` bên trong, nên khoá chặn kiểu thường là đúng và nhanh hơn.
    clients: Mutex<HashMap<K, Instant>>,
    requests: AtomicU64,
}

impl<K: Eq + std::hash::Hash + Copy> Gcra<K> {
    /// `None` when disabled (`per_minute = 0`).
    fn new(limit: RateLimit) -> Option<Self> {
        if limit.per_minute == 0 {
            return None;
        }
        let interval = Duration::from_secs(60) / limit.per_minute;
        Some(Gcra {
            interval,
            tolerance: interval * (limit.burst - 1),
            clients: Mutex::new(HashMap::new()),
            requests: AtomicU64::new(0),
        })
    }

    fn check(&self, client: K, now: Instant) -> Decision {
        let Ok(mut clients) = self.clients.lock() else {
            // Khoá chỉ "hỏng" khi một thread panic lúc đang giữ nó; không chặn ai cả
            // còn hơn chặn tất cả.
            return Decision::Allow;
        };
        if self.requests.fetch_add(1, Ordering::Relaxed).is_multiple_of(PRUNE_EVERY) {
            // Mốc đã lùi về quá khứ thì client như mới: bỏ đi cho bảng không phình.
            clients.retain(|_, tat| *tat > now);
        }
        let tat = clients.get(&client).copied().unwrap_or(now).max(now);
        let ahead = tat - now;
        if ahead > self.tolerance {
            return Decision::Deny(ahead - self.tolerance);
        }
        clients.insert(client, tat + self.interval);
        Decision::Allow
    }
}

/// The outcome of evaluating a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Decision {
    Allow,
    /// Rejected; the client should wait this long.
    Deny(Duration),
}

impl RateLimiter {
    /// `None` when rate limiting is disabled (`per_minute = 0`).
    pub(super) fn new(limit: RateLimit) -> Option<Self> {
        Gcra::new(limit).map(|cells| RateLimiter { cells })
    }

    /// Evaluates one request from `client` at time `now`.
    pub(super) fn check(&self, client: IpAddr, now: Instant) -> Decision {
        self.cells.check(client, now)
    }
}

/// Groups IPv6 addresses by /64 block: a machine usually owns the whole block, and changing address within
/// the block does not count as a new client. An IPv4 address written as IPv6 (`::ffff:a.b.c.d`, when the
/// server listens on `[::]`) is treated as IPv4 - otherwise every IPv4 client would fall into the same
/// `::/64` block and one client would get the whole group blocked (D-157).
fn group(address: IpAddr) -> IpAddr {
    match address.to_canonical() {
        IpAddr::V6(v6) => {
            let segments = v6.segments();
            IpAddr::V6(Ipv6Addr::new(
                segments[0],
                segments[1],
                segments[2],
                segments[3],
                0,
                0,
                0,
                0,
            ))
        }
        v4 => v4,
    }
}

/// The per-API-key limit (D-109): wraps the authenticator, counting one request each time an API key is
/// authenticated (REST's `Principal` extractor, MCP's authentication layer - once per request). A request
/// using a login session is not counted here (the per-address limit already applies).
pub(super) struct ApiKeyLimited {
    inner: Arc<dyn Authenticator>,
    counter: KeyCounter,
}

/// Where the per-API-key limit is counted.
enum KeyCounter {
    /// Each process counts separately.
    Memory(Gcra<Uuid>),
    /// Counted jointly through the database, in batches (D-375).
    Database(super::rate_limit_store::SharedCounter),
}

impl ApiKeyLimited {
    /// Wraps `inner` with the limit `limit`, counted in memory; when disabled (`per_minute = 0`) it returns
    /// `inner` unchanged.
    #[cfg(test)]
    fn wrap(inner: Arc<dyn Authenticator>, limit: RateLimit) -> Arc<dyn Authenticator> {
        Self::wrap_in(inner, limit, None)
    }

    /// Wraps `inner` with the limit `limit`; with a `registry` it counts jointly through the default
    /// tenant's database, otherwise in memory. When disabled (`per_minute = 0`) it returns `inner`
    /// unchanged.
    pub(super) fn wrap_in(
        inner: Arc<dyn Authenticator>,
        limit: RateLimit,
        registry: Option<crate::tenancy::DatabaseRegistry>,
    ) -> Arc<dyn Authenticator> {
        let counter = match registry {
            Some(registry) => super::rate_limit_store::SharedCounter::new(limit, registry)
                .map(KeyCounter::Database),
            None => Gcra::new(limit).map(KeyCounter::Memory),
        };
        match counter {
            Some(counter) => Arc::new(ApiKeyLimited { inner, counter }),
            None => inner,
        }
    }
}

#[async_trait::async_trait]
impl Authenticator for ApiKeyLimited {
    async fn authenticate(
        &self,
        authorization: Option<&str>,
        hint: &crate::tenancy::TenantHint,
    ) -> Result<Principal, Error> {
        let principal = self.inner.authenticate(authorization, hint).await?;
        let Some(key) = principal.credential_id() else { return Ok(principal) };
        let decision = match &self.counter {
            KeyCounter::Memory(cells) => cells.check(key, Instant::now()),
            KeyCounter::Database(shared) => shared.check(principal.tenant_id(), key).await,
        };
        if let Decision::Deny(wait) = decision {
            metrics::counter!("http_api_key_rate_limited_total").increment(1);
            let seconds = wait.as_secs() + u64::from(wait.subsec_nanos() > 0);
            return Err(Error::too_many_requests(
                "rate_limited",
                "Too many requests with this API key. Wait {seconds} seconds, then retry; send \
                 requests more slowly to avoid this.",
            )
            .with_param("seconds", seconds)
            .with_retry_after(seconds));
        }
        Ok(principal)
    }

    async fn enter(
        &self,
        principal: &Principal,
        target: &crate::tenancy::TenantContext,
    ) -> Result<Option<Principal>, Error> {
        self.inner.enter(principal, target).await
    }

    fn decides_entry(&self) -> bool {
        self.inner.decides_entry()
    }

    async fn principal_of(
        &self,
        tenant: &crate::tenancy::TenantContext,
        user_id: uuid::Uuid,
    ) -> Result<Option<Principal>, Error> {
        self.inner.principal_of(tenant, user_id).await
    }
}

/// The HTTP layer: rejects with 429 `rate_limited` and `Retry-After`.
pub(super) async fn limit(
    limiter: Option<Arc<RateLimiter>>,
    request: Request,
    next: Next,
) -> Response {
    let Some(limiter) = limiter else {
        return next.run(request).await;
    };
    // Health check do hệ thống vận hành gọi liên tục: không đếm.
    if request.uri().path().starts_with("/health/") {
        return next.run(request).await;
    }
    // Địa chỉ do lớp `client::attach` giải (theo số proxy tin cậy). Không biết địa chỉ
    // client (vd request không đi qua socket) thì không giới hạn.
    let Some(client) = request.extensions().get::<ClientIp>().and_then(|ip| ip.0).map(group) else {
        return next.run(request).await;
    };
    match limiter.check(client, Instant::now()) {
        Decision::Allow => next.run(request).await,
        Decision::Deny(wait) => {
            metrics::counter!("http_rate_limited_total").increment(1);
            // Làm tròn lên: chờ đúng số giây báo thì chắc chắn được nhận.
            let seconds = wait.as_secs() + u64::from(wait.subsec_nanos() > 0);
            Error::too_many_requests(
                "rate_limited",
                "Too many requests from your address. Wait {seconds} seconds, then retry; send \
                 requests more slowly to avoid this.",
            )
            .with_param("seconds", seconds)
            .with_retry_after(seconds)
            .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limiter(per_minute: u32, burst: u32) -> RateLimiter {
        RateLimiter::new(RateLimit { per_minute, burst }).unwrap()
    }

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    /// A fake authenticator: the token is the API key's ID (or `session` for a login session).
    struct Keys;

    #[async_trait::async_trait]
    impl Authenticator for Keys {
        async fn authenticate(
            &self,
            authorization: Option<&str>,
            _: &crate::tenancy::TenantHint,
        ) -> Result<Principal, Error> {
            let principal = Principal::new(Uuid::from_u128(1), Vec::<String>::new());
            Ok(match authorization {
                Some("session") | None => principal,
                Some(key) => principal.via_api_key(key.parse().map_err(Error::internal)?),
            })
        }
    }

    #[tokio::test]
    async fn api_key_dem_rieng_tung_key_phien_dang_nhap_khong_dem() {
        let limited = ApiKeyLimited::wrap(Arc::new(Keys), RateLimit { per_minute: 60, burst: 2 });
        let (a, b) = (Uuid::from_u128(10).to_string(), Uuid::from_u128(11).to_string());
        for _ in 0..2 {
            limited.authenticate(Some(&a), &crate::tenancy::TenantHint::none()).await.unwrap();
        }
        let error =
            limited.authenticate(Some(&a), &crate::tenancy::TenantHint::none()).await.unwrap_err();
        assert_eq!((error.code(), error.retry_after()), ("rate_limited", Some(1)));
        // Key khác vẫn chạy; phiên đăng nhập không bị đếm.
        limited.authenticate(Some(&b), &crate::tenancy::TenantHint::none()).await.unwrap();
        for _ in 0..5 {
            limited
                .authenticate(Some("session"), &crate::tenancy::TenantHint::none())
                .await
                .unwrap();
        }
        // Tắt: trả nguyên bộ xác thực.
        let off = ApiKeyLimited::wrap(Arc::new(Keys), RateLimit { per_minute: 0, burst: 1 });
        for _ in 0..5 {
            off.authenticate(Some(&a), &crate::tenancy::TenantHint::none()).await.unwrap();
        }
    }

    #[test]
    fn tat_thi_khong_co_bo_gioi_han() {
        assert!(RateLimiter::new(RateLimit { per_minute: 0, burst: 10 }).is_none());
    }

    #[test]
    fn cho_gui_don_toi_burst_roi_theo_toc_do() {
        // 60 request/phút = 1 request/giây, gửi dồn được 3.
        let limiter = limiter(60, 3);
        let start = Instant::now();
        let client = ip("203.0.113.7");

        for _ in 0..3 {
            assert_eq!(limiter.check(client, start), Decision::Allow);
        }
        assert_eq!(limiter.check(client, start), Decision::Deny(Duration::from_secs(1)));
        // Một giây sau có thêm đúng một lượt.
        let later = start + Duration::from_secs(1);
        assert_eq!(limiter.check(client, later), Decision::Allow);
        assert!(matches!(limiter.check(client, later), Decision::Deny(_)));
        // Client khác không bị ảnh hưởng.
        assert_eq!(limiter.check(ip("203.0.113.8"), start), Decision::Allow);
    }

    #[test]
    fn ipv6_cung_khoi_64_tinh_la_mot_client() {
        assert_eq!(group(ip("2001:db8:1:2:aaaa::1")), group(ip("2001:db8:1:2:bbbb::9")));
        assert_ne!(group(ip("2001:db8:1:2::1")), group(ip("2001:db8:1:3::1")));
        assert_eq!(group(ip("192.0.2.1")), ip("192.0.2.1"));
        // IPv4 viết dạng IPv6: mỗi địa chỉ IPv4 vẫn là một client (D-157).
        assert_eq!(group(ip("::ffff:192.0.2.1")), ip("192.0.2.1"));
        assert_ne!(group(ip("::ffff:192.0.2.1")), group(ip("::ffff:192.0.2.2")));
    }

    #[test]
    fn client_nguoi_duoc_don_khoi_bang() {
        let limiter = limiter(60, 1);
        let start = Instant::now();
        limiter.check(ip("192.0.2.1"), start);
        // Lần dọn chạy ở request thứ PRUNE_EVERY + 1, lúc client cũ đã nguội.
        limiter.cells.requests.store(PRUNE_EVERY, Ordering::Relaxed);

        limiter.check(ip("192.0.2.2"), start + Duration::from_secs(5));

        let clients = limiter.cells.clients.lock().unwrap();
        assert!(!clients.contains_key(&ip("192.0.2.1")));
        assert!(clients.contains_key(&ip("192.0.2.2")));
    }
}
