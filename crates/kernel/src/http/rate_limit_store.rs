//! Đếm giới hạn theo API key CHUNG giữa mọi máy chủ, qua bảng `kernel_throttle` (bước 40 P6, D-375).
//!
//! Bật bằng `AUTOXIDE_HTTP_RATE_LIMIT_STORE=database`. Để không thêm một câu ghi cho mỗi request,
//! đếm **theo lô**: mỗi tiến trình lấy một lô `k = max(1, burst / 4)` lượt của một key bằng
//! [`Throttle::take`] (một câu SQL), tiêu dần trong bộ nhớ, hết mới hỏi lại. Cửa sổ cố định 60 giây,
//! giới hạn `max(per_minute, burst)` lượt mỗi cửa sổ; sai số tối đa `k × số máy chủ` mỗi cửa sổ
//! (lượt đã lấy về một máy mà máy đó chưa dùng hết).
//!
//! Database không trả lời thì cho qua (ghi log, đếm `http_rate_limit_store_errors_total`): bộ giới hạn
//! không được làm sự cố database nặng thêm; request vẫn hỏng ở bước sau nếu database thật sự chết.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use uuid::Uuid;

use super::rate_limit::Decision;
use crate::config::RateLimit;
use crate::tenancy::{DatabaseRegistry, TenantId};
use crate::throttle::Throttle;

/// Cửa sổ đếm.
const WINDOW: Duration = Duration::from_secs(60);
/// Dọn dòng cũ của bảng đếm tối đa chừng này một lần mỗi tiến trình.
const PURGE_EVERY: Duration = Duration::from_secs(3600);
/// Dọn lô đã hết hạn khỏi bộ nhớ sau mỗi chừng này lần xét.
const PRUNE_EVERY: u64 = 1024;

/// Lô lượt của một key trong tiến trình này.
#[derive(Default)]
struct Lot {
    /// Lượt còn dùng được.
    left: u32,
    /// Hết hạn lúc cửa sổ của database kết thúc.
    until: Option<Instant>,
    /// Database đã cấp 0 lượt: chặn tới `until` mà không hỏi lại.
    denied: bool,
}

/// Lô của từng key (khách, API key).
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
    /// `None` khi tắt (`per_minute = 0`).
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

    /// Xét một request của API key `key` thuộc khách `tenant`.
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

    /// Dọn dòng đếm cũ (mọi khách) tối đa mỗi giờ một lần, chạy nền — không cần job theo lịch
    /// (như nhật ký request, D-370).
    fn purge_now_and_then(&self, now: Instant) {
        let Ok(mut last) = self.last_purge.lock() else { return };
        if last.is_some_and(|last| now.duration_since(last) < PURGE_EVERY) {
            return;
        }
        *last = Some(now);
        let throttle = self.throttle.clone();
        let database = self.registry.default_tenant().database().clone();
        tokio::spawn(async move {
            let before = chrono::Utc::now() - chrono::Duration::minutes(10);
            if let Err(error) = throttle.purge(&database, before).await {
                tracing::warn!(error = %error, "purging old API key rate limit counters failed");
            }
        });
    }
}
