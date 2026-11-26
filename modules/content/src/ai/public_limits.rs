//! Chốt của tìm công khai (PT-15): tìm theo chữ trên bản đã đăng là `LIKE '%q%'` quét mọi bài đã đăng của ngôn ngữ, nên
//! vài chục request không đăng nhập cùng lúc làm chậm mọi request khác. Ba chốt, trong bộ nhớ của từng tiến trình:
//!
//! - số lượt tìm công khai chạy cùng lúc (`AUTOXIDE_CONTENT_PUBLIC_SEARCH_CONCURRENCY`): hết chỗ thì trả 429 ngay, không
//!   xếp hàng giữ kết nối;
//! - số lượt mỗi phút của một địa chỉ (`AUTOXIDE_CONTENT_PUBLIC_SEARCH_PER_ADDRESS_PER_MINUTE`, theo
//!   `throttle::address_key`: một IPv4, hay một khối /64 của IPv6);
//! - trần thời gian của một lượt ([`TIMEOUT`]): quá thì bỏ dở, trả 503.
//!
//! Đếm trong bộ nhớ là đủ cho mục đích này (giữ máy chủ không nghẽn); nhiều tiến trình thì mỗi tiến trình một chốt.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use autoxide_kernel::error::Error;
use autoxide_kernel::throttle::address_key;

/// Trần thời gian của một lượt tìm công khai.
pub(crate) const TIMEOUT: Duration = Duration::from_secs(3);
/// Cửa sổ đếm theo địa chỉ.
const WINDOW: Duration = Duration::from_secs(60);
/// Bảng đếm lớn hơn chừng này thì dọn các cửa sổ đã qua (không dọn mỗi lượt).
const SWEEP_ABOVE: usize = 1024;

#[derive(Default)]
pub(crate) struct PublicLimits {
    running: AtomicU32,
    // `Mutex` của thư viện chuẩn: giữ khoá rất ngắn, không có `await` bên trong.
    by_address: Mutex<HashMap<String, (Instant, u32)>>,
}

/// Một chỗ chạy đang giữ; trả lại khi bị huỷ (`Drop`) — kể cả khi lượt tìm lỗi hay quá giờ.
pub(crate) struct Slot<'a>(&'a AtomicU32);

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

impl PublicLimits {
    /// Tính một lượt của địa chỉ `ip` (không biết địa chỉ: không đếm) và giữ một chỗ chạy; lỗi 429 khi vượt.
    pub(crate) fn admit(
        &self,
        ip: Option<IpAddr>,
        per_address: u32,
        concurrency: u32,
    ) -> Result<Slot<'_>, Error> {
        if let Some(ip) = ip {
            self.count(&address_key("content-public-search", ip), per_address)?;
        }
        // Tăng trước rồi kiểm: hai lượt cùng lúc không cùng thấy còn một chỗ.
        if self.running.fetch_add(1, Ordering::AcqRel) >= concurrency {
            self.running.fetch_sub(1, Ordering::AcqRel);
            return Err(Error::too_many_requests(
                "content.public_search_busy",
                "Too many searches are running right now. Try again in a moment.",
            )
            .with_retry_after(1));
        }
        Ok(Slot(&self.running))
    }

    fn count(&self, key: &str, limit: u32) -> Result<(), Error> {
        let now = Instant::now();
        let Ok(mut counts) = self.by_address.lock() else { return Ok(()) };
        if counts.len() > SWEEP_ABOVE {
            counts.retain(|_, (start, _)| now.duration_since(*start) < WINDOW);
        }
        let entry = counts.entry(key.to_owned()).or_insert((now, 0));
        if now.duration_since(entry.0) >= WINDOW {
            *entry = (now, 0);
        }
        if entry.1 >= limit {
            let wait = WINDOW.saturating_sub(now.duration_since(entry.0)).as_secs().max(1);
            return Err(Error::too_many_requests(
                "content.public_search_rate_limited",
                "Too many searches from your network. Try again in {seconds} seconds.",
            )
            .with_param("seconds", wait)
            .with_retry_after(wait));
        }
        entry.1 += 1;
        Ok(())
    }
}

/// Lỗi khi một lượt tìm công khai vượt [`TIMEOUT`].
pub(crate) fn timed_out() -> Error {
    Error::unavailable(
        "content.public_search_timeout",
        "The search took too long and was stopped. Try a more specific query.",
    )
    .with_retry_after(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> Option<IpAddr> {
        text.parse().ok()
    }

    #[test]
    fn het_cho_hay_qua_luot_thi_429() {
        let limits = PublicLimits::default();
        // Hai chỗ: lượt thứ ba bị từ chối khi hai lượt trước còn chạy; trả chỗ thì nhận lại.
        let first = limits.admit(ip("203.0.113.1"), 100, 2).unwrap();
        let second = limits.admit(ip("203.0.113.2"), 100, 2).unwrap();
        let busy = limits.admit(ip("203.0.113.3"), 100, 2).err().unwrap();
        assert_eq!(busy.code(), "content.public_search_busy");
        drop(first);
        assert!(limits.admit(ip("203.0.113.3"), 100, 2).is_ok());
        drop(second);
        // Theo địa chỉ: 3 lượt mỗi phút; cùng khối /64 của IPv6 tính chung, địa chỉ khác không bị ảnh hưởng.
        for _ in 0..3 {
            drop(limits.admit(ip("2001:db8:1:2::1"), 3, 10).unwrap());
        }
        let limited = limits.admit(ip("2001:db8:1:2::99"), 3, 10).err().unwrap();
        assert_eq!(limited.code(), "content.public_search_rate_limited");
        assert!(limited.retry_after().is_some());
        assert!(limits.admit(ip("2001:db8:1:3::1"), 3, 10).is_ok());
    }
}
