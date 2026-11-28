//! The `RateLimit-Policy` and `RateLimit` response headers (draft-ietf-httpapi-ratelimit-headers-12, November
//! 2026): a client learns its quota before it is refused, and on a 429 which policy refused it.
//!
//! - `RateLimit-Policy: "<name>";q=<quota>;w=<window seconds>` - the policy.
//! - `RateLimit: "<name>";a=<available>;w=<seconds>` - what is left now, and the seconds until the whole quota
//!   is back (on a 429: the seconds to wait, the same as `Retry-After`).
//!
//! Policies: `address` (per client address, on every response), `api-key` and `tenant` (on their 429 only:
//! they are counted after authentication, deeper than this header layer). Both headers are Structured Field
//! lists, so several limits add list members.

use axum::http::{HeaderMap, HeaderName, HeaderValue};

use crate::config::RateLimit;
use crate::error::RateLimitPolicy;

/// `RateLimit-Policy`.
pub(crate) const RATE_LIMIT_POLICY: HeaderName = HeaderName::from_static("ratelimit-policy");
/// `RateLimit`.
pub(crate) const RATE_LIMIT: HeaderName = HeaderName::from_static("ratelimit");

/// The policy of a limit counted by GCRA in memory: `burst` requests, refilled one every
/// `60 / per_minute` seconds, so the whole quota is back after `burst × 60 / per_minute` seconds.
//
// Báo thấp hơn thật một chút (trong `w` giây, từ lúc đầy, client gửi được tới `2 × burst − 1` vì hạn mức đầy lại
// dần): an toàn cho client, và `q` / `w` nói đúng điều quan trọng — dồn được bao nhiêu, bao lâu thì đầy lại.
pub(crate) fn gcra_policy(name: &'static str, limit: RateLimit) -> RateLimitPolicy {
    let burst = limit.burst.max(1);
    let per_minute = u64::from(limit.per_minute.max(1));
    // Làm tròn lên, ít nhất 1 giây: `w` của bản nháp là số nguyên dương.
    let window = (u64::from(burst) * 60).div_ceil(per_minute).max(1);
    RateLimitPolicy { name, quota: burst, window }
}

/// The policy of a limit counted through the database: fixed one-minute windows of `max(per_minute, burst)`
/// requests (see `rate_limit_store`).
pub(crate) fn window_policy(name: &'static str, limit: RateLimit) -> RateLimitPolicy {
    RateLimitPolicy { name, quota: limit.per_minute.max(limit.burst), window: 60 }
}

/// Appends `policy` to `RateLimit-Policy` and its current state to `RateLimit`: `available` requests left,
/// whole quota back (or, with `available = 0`, the next request accepted) in `seconds`.
pub(crate) fn append(
    headers: &mut HeaderMap,
    policy: RateLimitPolicy,
    available: u32,
    seconds: u64,
) {
    let RateLimitPolicy { name, quota, window } = policy;
    // Tên chính sách là hằng chữ thường không dấu: luôn là giá trị header hợp lệ.
    if let Ok(value) = HeaderValue::from_str(&format!("\"{name}\";q={quota};w={window}")) {
        headers.append(RATE_LIMIT_POLICY, value);
    }
    if let Ok(value) = HeaderValue::from_str(&format!("\"{name}\";a={available};w={seconds}")) {
        headers.append(RATE_LIMIT, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chinh_sach_gcra_va_cua_so_co_dinh() {
        // 600/phút, dồn 60: 60 request, đầy lại sau 6 giây.
        let limit = RateLimit { per_minute: 600, burst: 60 };
        assert_eq!(
            gcra_policy("address", limit),
            RateLimitPolicy { name: "address", quota: 60, window: 6 }
        );
        // 1/phút, dồn 1: 60 giây; cửa sổ không bao giờ là 0.
        let slow = RateLimit { per_minute: 7, burst: 1 };
        assert_eq!(gcra_policy("tenant", slow).window, 9);
        let fast = RateLimit { per_minute: 6000, burst: 1 };
        assert_eq!(gcra_policy("tenant", fast).window, 1);
        assert_eq!(
            window_policy("api-key", RateLimit { per_minute: 120, burst: 30 }),
            RateLimitPolicy { name: "api-key", quota: 120, window: 60 }
        );
    }

    #[test]
    fn hai_chinh_sach_thanh_hai_phan_tu_danh_sach() {
        let mut headers = HeaderMap::new();
        append(&mut headers, RateLimitPolicy { name: "api-key", quota: 30, window: 60 }, 0, 12);
        append(&mut headers, RateLimitPolicy { name: "address", quota: 60, window: 6 }, 59, 1);
        let all = |name| {
            headers.get_all(name).iter().map(|v| v.to_str().unwrap().to_owned()).collect::<Vec<_>>()
        };
        assert_eq!(all(RATE_LIMIT_POLICY), ["\"api-key\";q=30;w=60", "\"address\";q=60;w=6"]);
        assert_eq!(all(RATE_LIMIT), ["\"api-key\";a=0;w=12", "\"address\";a=59;w=1"]);
    }
}
