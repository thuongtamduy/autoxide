//! Sổ hãng vận chuyển: bộ nối (module đăng ký SAU `shipping`) gọi `register` trong
//! `Module::start`. Giữ trong bộ nhớ của tiến trình; đăng ký chỉ xảy ra lúc khởi động.

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError, RwLock};

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::TenantContext;

use crate::api::fake::FakeCarrier;
use crate::api::{CarrierRegistry, ShippingCarrier};

pub(crate) struct Registry {
    carriers: RwLock<BTreeMap<&'static str, Arc<dyn ShippingCarrier>>>,
    /// Hãng giả mọi sandbox dùng thay hãng thật (bước 38, D-232): không gọi mạng, không tạo vận đơn
    /// thật.
    simulated: Arc<FakeCarrier>,
}

impl Default for Registry {
    fn default() -> Self {
        Registry { carriers: RwLock::default(), simulated: Arc::new(FakeCarrier::new()) }
    }
}

impl Registry {
    /// Bộ nối của hãng `code` cho khách `tenant`; sandbox luôn nhận hãng giả.
    pub(crate) fn for_tenant(
        &self,
        tenant: &TenantContext,
        code: &str,
    ) -> Option<Arc<dyn ShippingCarrier>> {
        let real = self.get(code)?;
        if tenant.is_sandbox() {
            return Some(self.simulated.clone());
        }
        Some(real)
    }
}

impl CarrierRegistry for Registry {
    fn register(&self, carrier: Arc<dyn ShippingCarrier>) -> Result<(), Error> {
        let code = carrier.code();
        // Khoá "nhiễm độc" chỉ khi một luồng panic lúc giữ khoá: bản đồ vẫn dùng được.
        let mut carriers = self.carriers.write().unwrap_or_else(PoisonError::into_inner);
        if carriers.contains_key(code) {
            return Err(Error::internal(format!(
                "shipping carrier `{code}` is registered twice: two connector modules use the same code"
            )));
        }
        carriers.insert(code, carrier);
        Ok(())
    }

    fn get(&self, code: &str) -> Option<Arc<dyn ShippingCarrier>> {
        self.carriers.read().unwrap_or_else(PoisonError::into_inner).get(code).cloned()
    }

    fn codes(&self) -> Vec<&'static str> {
        self.carriers.read().unwrap_or_else(PoisonError::into_inner).keys().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::Registry;
    use crate::api::CarrierRegistry;
    use crate::api::fake::FakeCarrier;

    #[test]
    fn dang_ky_trung_ma_bi_tu_choi() {
        let registry = Registry::default();
        registry.register(Arc::new(FakeCarrier::new())).unwrap();
        assert!(registry.register(Arc::new(FakeCarrier::new())).is_err());
        assert_eq!(registry.codes(), vec!["fake"]);
        assert!(registry.get("fake").is_some());
        assert!(registry.get("ghn").is_none());
    }
}
