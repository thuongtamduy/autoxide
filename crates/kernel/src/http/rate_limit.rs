//! Rate limiting: giới hạn số request của mỗi địa chỉ client.
//!
//! Thuật toán GCRA (Generic Cell Rate Algorithm — cách tính của "token bucket"
//! nhưng chỉ cần nhớ MỘT mốc thời gian cho mỗi client): mỗi request đẩy mốc
//! "thời điểm lý thuyết của request kế" (`tat`) đi thêm một khoảng `interval`
//! (= 1 phút / số request mỗi phút). Request bị từ chối khi mốc đó đã vượt quá hiện
//! tại hơn `tolerance` (= `interval` × (burst − 1)) — tức client đã dùng hết phần gửi
//! dồn và đang gửi nhanh hơn tốc độ cho phép.
//!
//! Đếm trong bộ nhớ của từng tiến trình: chạy nhiều máy chủ thì mỗi máy đếm riêng.
//! Đủ để chặn một client gửi dồn; chặn đoán mật khẩu thì nằm ở login throttling
//! (đếm trong database, dùng chung mọi máy chủ).

use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Request};
use axum::http::HeaderName;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::config::RateLimit;
use crate::error::Error;

/// Dọn các client đã "nguội" sau mỗi chừng này request.
const PRUNE_EVERY: u64 = 1024;

pub(super) struct RateLimiter {
    interval: Duration,
    tolerance: Duration,
    client_ip_header: Option<HeaderName>,
    // `Mutex` của thư viện chuẩn (không phải của tokio): đoạn giữ khoá rất ngắn và
    // không có `await` bên trong, nên khoá chặn kiểu thường là đúng và nhanh hơn.
    clients: Mutex<HashMap<IpAddr, Instant>>,
    requests: AtomicU64,
}

/// Kết quả xét một request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Decision {
    Allow,
    /// Từ chối; client nên chờ chừng này.
    Deny(Duration),
}

impl RateLimiter {
    /// `None` khi rate limiting bị tắt (`per_minute = 0`).
    pub(super) fn new(limit: RateLimit, client_ip_header: Option<HeaderName>) -> Option<Self> {
        if limit.per_minute == 0 {
            return None;
        }
        let interval = Duration::from_secs(60) / limit.per_minute;
        Some(RateLimiter {
            interval,
            tolerance: interval * (limit.burst - 1),
            client_ip_header,
            clients: Mutex::new(HashMap::new()),
            requests: AtomicU64::new(0),
        })
    }

    /// Xét một request của `client` tại thời điểm `now`.
    pub(super) fn check(&self, client: IpAddr, now: Instant) -> Decision {
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

    /// Địa chỉ client của request: từ header của reverse proxy nếu được cấu hình
    /// (phần tử cuối — do proxy của mình thêm vào), không thì từ kết nối.
    fn client(&self, request: &Request) -> Option<IpAddr> {
        let from_header = self.client_ip_header.as_ref().and_then(|name| {
            // DÒNG CUỐI của header (proxy có thể thêm dòng mới thay vì nối vào dòng client
            // gửi), phần tử cuối của dòng đó — phần do proxy của mình ghi (D-079).
            let line = request.headers().get_all(name).iter().next_back()?.to_str().ok()?;
            let last = line.rsplit(',').next()?.trim();
            let parsed = parse_address(last);
            if parsed.is_none() {
                tracing::warn!(
                    header = %name,
                    value = last,
                    "cannot read the client address from the configured header; using the \
                     connection address"
                );
            }
            parsed
        });
        let from_connection =
            || request.extensions().get::<ConnectInfo<SocketAddr>>().map(|info| info.0.ip());
        from_header.or_else(from_connection).map(group)
    }
}

/// Địa chỉ IP trong header của proxy: `203.0.113.9`, `203.0.113.9:4711`, `2001:db8::1`
/// hoặc `[2001:db8::1]:4711` (vài load balancer ghi kèm port).
fn parse_address(text: &str) -> Option<IpAddr> {
    text.parse::<IpAddr>()
        .ok()
        .or_else(|| text.parse::<SocketAddr>().ok().map(|address| address.ip()))
}

/// Gom địa chỉ IPv6 theo khối /64: một máy thường có cả khối, đổi địa chỉ trong khối
/// không được tính là client mới.
fn group(address: IpAddr) -> IpAddr {
    match address {
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

/// Lớp HTTP: từ chối bằng 429 `rate_limited` kèm `Retry-After`.
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
    // Không biết địa chỉ client (vd request không đi qua socket) thì không giới hạn.
    let Some(client) = limiter.client(&request) else {
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
        RateLimiter::new(RateLimit { per_minute, burst }, None).unwrap()
    }

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn tat_thi_khong_co_bo_gioi_han() {
        assert!(RateLimiter::new(RateLimit { per_minute: 0, burst: 10 }, None).is_none());
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
    }

    #[test]
    fn client_nguoi_duoc_don_khoi_bang() {
        let limiter = limiter(60, 1);
        let start = Instant::now();
        limiter.check(ip("192.0.2.1"), start);
        // Lần dọn chạy ở request thứ PRUNE_EVERY + 1, lúc client cũ đã nguội.
        limiter.requests.store(PRUNE_EVERY, Ordering::Relaxed);

        limiter.check(ip("192.0.2.2"), start + Duration::from_secs(5));

        let clients = limiter.clients.lock().unwrap();
        assert!(!clients.contains_key(&ip("192.0.2.1")));
        assert!(clients.contains_key(&ip("192.0.2.2")));
    }

    #[test]
    fn dia_chi_lay_tu_header_cua_proxy_neu_duoc_cau_hinh() {
        let header = HeaderName::from_static("x-forwarded-for");
        let behind_proxy =
            RateLimiter::new(RateLimit { per_minute: 60, burst: 1 }, Some(header)).unwrap();
        let direct = limiter(60, 1);
        let mut request = Request::builder()
            .header("x-forwarded-for", "198.51.100.1, 203.0.113.9")
            .body(axum::body::Body::empty())
            .unwrap();
        request.extensions_mut().insert(ConnectInfo(SocketAddr::from(([10, 0, 0, 1], 4000))));

        assert_eq!(behind_proxy.client(&request), Some(ip("203.0.113.9")));
        assert_eq!(direct.client(&request), Some(ip("10.0.0.1")));

        // Client tự gửi một dòng giả, proxy thêm dòng mới: lấy dòng của proxy (dòng cuối).
        let mut forged = Request::builder()
            .header("x-forwarded-for", "1.2.3.4")
            .header("x-forwarded-for", "203.0.113.10")
            .body(axum::body::Body::empty())
            .unwrap();
        forged.extensions_mut().insert(ConnectInfo(SocketAddr::from(([10, 0, 0, 1], 4000))));
        assert_eq!(behind_proxy.client(&forged), Some(ip("203.0.113.10")));
    }

    #[test]
    fn dia_chi_kem_port_van_doc_duoc() {
        assert_eq!(parse_address("203.0.113.9:4711"), Some(ip("203.0.113.9")));
        assert_eq!(parse_address("[2001:db8::1]:4711"), Some(ip("2001:db8::1")));
        assert_eq!(parse_address("2001:db8::1"), Some(ip("2001:db8::1")));
        assert_eq!(parse_address("unknown"), None);
    }
}
