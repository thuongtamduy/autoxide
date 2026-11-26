//! Chốt của tìm công khai (PT-15): tìm theo ý nghĩa tốn CPU và gọi nhà cung cấp; tìm theo chữ đi qua chỉ mục từ có trần
//! thời gian câu lệnh riêng (`publishing::words`, D-20261126-content-word-index). Ba chốt, trong bộ nhớ của từng
//! tiến trình:
//!
//! - số lượt tìm công khai chạy cùng lúc (`AUTOXIDE_CONTENT_PUBLIC_SEARCH_CONCURRENCY`): hết chỗ thì trả 429 ngay, không
//!   xếp hàng giữ kết nối;
//! - số lượt mỗi phút của một địa chỉ (`AUTOXIDE_CONTENT_PUBLIC_SEARCH_PER_ADDRESS_PER_MINUTE`, theo
//!   `throttle::address_key`: một IPv4, hay một khối /64 của IPv6);
//! - trần thời gian trả lời ([`TIMEOUT`]): quá thì trả 503. Lượt tìm chạy trong task riêng giữ chỗ tới khi việc xong
//!   thật (câu truy vấn đang chạy, việc tính vector) — chỗ phản ánh tải thật, không phải số request đang chờ trả lời.
//!   Phần tìm theo ý nghĩa có trần riêng ngắn hơn ([`MEANING_BUDGET`]); quá thì lùi về kết quả theo chữ.
//!
//! Đếm trong bộ nhớ là đủ cho mục đích này (giữ máy chủ không nghẽn); nhiều tiến trình thì mỗi tiến trình một chốt.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use autoxide_kernel::error::Error;
use autoxide_kernel::throttle::address_key;

/// Trần thời gian trả lời một lượt tìm công khai.
pub(crate) const TIMEOUT: Duration = Duration::from_secs(3);
/// Trần của phần tìm theo ý nghĩa (gọi mô hình tính vector, nạp index): quá thì chỉ trả kết quả theo chữ.
pub(crate) const MEANING_BUDGET: Duration = Duration::from_millis(1500);
/// Cửa sổ đếm theo địa chỉ.
const WINDOW: Duration = Duration::from_secs(60);
/// Bảng đếm có chừng này mục thì dọn các cửa sổ đã qua — tối đa mỗi giây một lần, không mỗi lượt.
const SWEEP_ABOVE: usize = 1024;
/// Trần cứng số địa chỉ đang đếm: vẫn đầy sau khi dọn thì bắt đầu lại bảng (rất hiếm; người dùng nhiều khối địa chỉ
/// tới vậy đã có nhiều lượt hơn trần theo địa chỉ cho mỗi khối).
const MAX_ADDRESSES: usize = 100_000;

#[derive(Default)]
pub(crate) struct PublicLimits {
    running: Arc<AtomicU32>,
    // `Mutex` của thư viện chuẩn: giữ khoá rất ngắn, không có `await` bên trong.
    by_address: Mutex<Counts>,
}

#[derive(Default)]
struct Counts {
    windows: HashMap<String, (Instant, u32)>,
    swept_at: Option<Instant>,
}

/// Một chỗ chạy đang giữ; trả lại khi bị huỷ (`Drop`) — kể cả khi lượt tìm lỗi hay panic. Sở hữu bộ đếm (`Arc`) để đi
/// theo task chạy lượt tìm tới khi việc xong, sau cả lúc request đã nhận 503 hay client đã ngắt.
pub(crate) struct Slot(Arc<AtomicU32>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

impl PublicLimits {
    /// Giữ một chỗ chạy rồi tính một lượt của địa chỉ `ip` (không biết địa chỉ: không đếm); lỗi 429 khi vượt. Hết chỗ thì
    /// không tính lượt của địa chỉ (người đọc không mất lượt vì máy chủ bận).
    pub(crate) fn admit(
        &self,
        ip: Option<IpAddr>,
        per_address: u32,
        concurrency: u32,
    ) -> Result<Slot, Error> {
        // Tăng trước rồi kiểm: hai lượt cùng lúc không cùng thấy còn một chỗ.
        if self.running.fetch_add(1, Ordering::AcqRel) >= concurrency {
            self.running.fetch_sub(1, Ordering::AcqRel);
            return Err(Error::too_many_requests(
                "content.public_search_busy",
                "Too many searches are running right now. Try again in a moment.",
            )
            .with_retry_after(1));
        }
        let slot = Slot(Arc::clone(&self.running));
        if let Some(ip) = ip {
            // Lỗi thì `slot` bị huỷ ở đây, chỗ được trả lại.
            self.count(&address_key("content-public-search", ip), per_address)?;
        }
        Ok(slot)
    }

    fn count(&self, key: &str, limit: u32) -> Result<(), Error> {
        let now = Instant::now();
        let Ok(mut counts) = self.by_address.lock() else { return Ok(()) };
        let Counts { windows, swept_at } = &mut *counts;
        if windows.len() >= SWEEP_ABOVE
            && swept_at.is_none_or(|at| now.duration_since(at) >= Duration::from_secs(1))
        {
            windows.retain(|_, (start, _)| now.duration_since(*start) < WINDOW);
            *swept_at = Some(now);
        }
        if windows.len() >= MAX_ADDRESSES && !windows.contains_key(key) {
            windows.clear();
        }
        let entry = windows.entry(key.to_owned()).or_insert((now, 0));
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

    #[test]
    fn het_cho_thi_khong_mat_luot_cua_dia_chi() {
        let limits = PublicLimits::default();
        let held = limits.admit(None, 1, 1).unwrap();
        for _ in 0..5 {
            let busy = limits.admit(ip("203.0.113.1"), 1, 1).err().unwrap();
            assert_eq!(busy.code(), "content.public_search_busy");
        }
        drop(held);
        // Năm lượt bị từ chối vì bận không tiêu lượt duy nhất của địa chỉ.
        assert!(limits.admit(ip("203.0.113.1"), 1, 1).is_ok());
    }

    #[test]
    fn bang_dem_co_tran() {
        let limits = PublicLimits::default();
        for index in 0..(MAX_ADDRESSES + 10) {
            let [a, b, c, d] = u32::try_from(index).unwrap().to_be_bytes();
            let address = IpAddr::from([10 | a, b, c, d]);
            drop(limits.admit(Some(address), 5, 1).unwrap());
        }
        assert!(limits.by_address.lock().unwrap().windows.len() <= MAX_ADDRESSES);
    }

    #[tokio::test]
    async fn cho_di_theo_task_toi_khi_viec_xong() {
        let limits = PublicLimits::default();
        let slot = limits.admit(None, 10, 1).unwrap();
        let (release, wait) = tokio::sync::oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            let _slot = slot;
            let _ = wait.await;
        });
        // Task còn chạy (request có thể đã trả 503): chỗ vẫn bị giữ.
        assert_eq!(limits.admit(None, 10, 1).err().unwrap().code(), "content.public_search_busy");
        release.send(()).unwrap();
        task.await.unwrap();
        assert!(limits.admit(None, 10, 1).is_ok());
    }
}
