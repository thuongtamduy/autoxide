//! Chỗ nối muộn (D-20261122-late-apis): lấy trait do module đăng ký SAU công bố.
//!
//! [`StartContext::require`](super::StartContext::require) chỉ thấy module đăng ký trước. Khi hai
//! module cần nhau theo chiều ngược thứ tự đăng ký (vd `auth` đăng ký trước nhưng cần một trait
//! `orders` công bố) thì module trước giữ một [`Later<T>`]: kernel điền nó sau khi MỌI module đã khởi
//! động, trước khi phục vụ request hay chạy job. Đọc chỉ lúc phục vụ, không lúc `start`.

use std::sync::{Arc, OnceLock};

/// Tay cầm tới trait `T` (kiểu `dyn Trait`) mà một module nào đó công bố bằng `provide`, kể cả module
/// đăng ký sau bên giữ. Không ai công bố (module tắt, chưa cài) → [`get`](Self::get) trả `None`: bên
/// giữ coi đó là "tính năng không có".
//
// `Arc<OnceLock<…>>`: kernel (bên điền) và module (bên đọc) giữ cùng một ô; `OnceLock` cho điền đúng
// một lần mà không cần khoá khi đọc. `T: ?Sized` cho phép `T = dyn Trait`.
pub struct Later<T: ?Sized> {
    slot: Arc<OnceLock<Option<Arc<T>>>>,
}

// Viết tay thay vì `#[derive(Clone)]`: derive đòi `T: Clone`, mà `dyn Trait` không có kích thước
// nên không Clone được; nhân bản tay cầm chỉ cần nhân bản `Arc`.
impl<T: ?Sized> Clone for Later<T> {
    fn clone(&self) -> Self {
        Later { slot: Arc::clone(&self.slot) }
    }
}

impl<T: ?Sized> std::fmt::Debug for Later<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Later")
            .field("contract", &std::any::type_name::<T>())
            .field("filled", &self.slot.get().map(Option::is_some))
            .finish()
    }
}

impl<T: ?Sized> Later<T> {
    /// Tay cầm trống, chưa điền. Kernel tạo qua `StartContext::later`; test tự dựng service thì
    /// dùng [`ready`](Self::ready).
    pub(crate) fn empty() -> Self {
        Later { slot: Arc::new(OnceLock::new()) }
    }

    /// Tay cầm đã điền sẵn (cho test hay binary tự ghép service).
    pub fn ready(api: Option<Arc<T>>) -> Self {
        let slot = OnceLock::new();
        let _ = slot.set(api);
        Later { slot: Arc::new(slot) }
    }

    /// Trait đã công bố, nếu có. Trước khi mọi module khởi động xong luôn là `None`.
    pub fn get(&self) -> Option<Arc<T>> {
        self.slot.get().and_then(Clone::clone)
    }

    /// Điền ô (kernel gọi đúng một lần sau khi mọi module khởi động).
    pub(crate) fn fill(&self, api: Option<Arc<T>>) {
        let _ = self.slot.set(api);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    trait Greeter: Send + Sync {
        fn hello(&self) -> &'static str;
    }

    struct English;

    impl Greeter for English {
        fn hello(&self) -> &'static str {
            "hello"
        }
    }

    #[test]
    fn chua_dien_thi_none_dien_mot_lan() {
        let later: Later<dyn Greeter> = Later::empty();
        let copy = later.clone();
        assert!(later.get().is_none());
        later.fill(Some(Arc::new(English)));
        assert_eq!(copy.get().map(|api| api.hello()), Some("hello"));
        // Điền lần hai không đổi gì.
        later.fill(None);
        assert!(copy.get().is_some());
        assert!(format!("{later:?}").contains("filled"));
    }

    #[test]
    fn ready_dung_ngay() {
        let later: Later<dyn Greeter> = Later::ready(None);
        assert!(later.get().is_none());
    }
}
