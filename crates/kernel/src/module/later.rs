//! Late binding (D-20261122-late-apis): fetches a trait published by a module registered LATER.
//!
//! [`StartContext::require`](super::StartContext::require) only sees modules registered earlier. When two
//! modules need each other against the registration order (e.g. `auth` is registered first but needs a
//! trait published by `orders`), the earlier module holds a [`Later<T>`]: the kernel fills it after EVERY
//! module has started, before serving requests or running jobs. Read it only while serving, not during
//! `start`.

use std::sync::{Arc, OnceLock};

/// A handle to the trait `T` (a `dyn Trait` type) that some module publishes with `provide`, including a
/// module registered after the holder. If nobody publishes it (module disabled, not installed),
/// [`get`](Self::get) returns `None`: the holder treats that as "feature not available".
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
    /// An empty handle, not yet filled. The kernel creates it through `StartContext::later`; tests that
    /// build a service themselves use [`ready`](Self::ready).
    pub(crate) fn empty() -> Self {
        Later { slot: Arc::new(OnceLock::new()) }
    }

    /// A handle that is already filled (for tests or binaries that assemble services themselves).
    pub fn ready(api: Option<Arc<T>>) -> Self {
        let slot = OnceLock::new();
        let _ = slot.set(api);
        Later { slot: Arc::new(slot) }
    }

    /// The published trait, if any. Always `None` before every module has finished starting.
    pub fn get(&self) -> Option<Arc<T>> {
        self.slot.get().and_then(Clone::clone)
    }

    /// Fills the slot (the kernel calls this exactly once after every module has started).
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
