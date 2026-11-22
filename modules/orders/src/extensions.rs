//! Cài đặt của điểm cắm [`OrderExtensions`]: module đăng ký sau `orders` (bước 46–49) cắm trait của
//! mình vào đây lúc khởi động.
//
// Vì sao cần: `Module::start` chạy theo thứ tự đăng ký, nên lúc `orders` khởi động thì `marketplace`,
// `customers` chưa chạy và chưa `provide` gì — `orders` không `require` được trait của họ. Ngược lại
// họ `require::<dyn OrderExtensions>()` được (orders đã chạy trước), rồi gọi `set_…` để trao trait
// của mình cho `orders`.
//
// Với người quen TypeScript: `Arc<dyn SellerSplit>` giống một tham chiếu tới object cài interface
// `SellerSplit`, đếm tham chiếu và dùng chung được giữa các thread. `OnceLock` là ô nhớ chỉ ghi được
// một lần rồi đọc mãi không cần khoá — đúng nhu cầu "cắm lúc khởi động, đọc lúc phục vụ request".

use std::sync::{Arc, Mutex, OnceLock};

use autoxide_kernel::error::Error;

use crate::api::{BuyerGroups, OrderExtensions, SellerSplit, Tender};

#[derive(Default)]
pub(crate) struct Extensions {
    seller_split: OnceLock<Arc<dyn SellerSplit>>,
    buyer_groups: OnceLock<Arc<dyn BuyerGroups>>,
    tenders: Mutex<Vec<Arc<dyn Tender>>>,
}

fn twice(point: &str) -> Error {
    Error::internal(format!("orders: the extension point `{point}` was plugged twice"))
}

impl Extensions {
    /// Nhóm khách, nếu `customers` đã cắm.
    pub(crate) fn buyer_groups(&self) -> Option<Arc<dyn BuyerGroups>> {
        self.buyer_groups.get().cloned()
    }

    /// Tender loại `kind`, nếu đã cắm.
    pub(crate) fn tender(&self, kind: &str) -> Option<Arc<dyn Tender>> {
        let tenders = self.tenders.lock().ok()?;
        tenders.iter().find(|tender| tender.kind() == kind).cloned()
    }

    /// Mọi tender đã cắm, theo thứ tự cắm.
    pub(crate) fn tenders(&self) -> Vec<Arc<dyn Tender>> {
        self.tenders.lock().map(|tenders| tenders.clone()).unwrap_or_default()
    }

    /// Loại của mọi tender đã cắm.
    pub(crate) fn tender_kinds(&self) -> Vec<&'static str> {
        self.tenders
            .lock()
            .map(|tenders| tenders.iter().map(|tender| tender.kind()).collect())
            .unwrap_or_default()
    }
}

impl OrderExtensions for Extensions {
    fn set_seller_split(&self, split: Arc<dyn SellerSplit>) -> Result<(), Error> {
        self.seller_split.set(split).map_err(|_| twice("seller_split"))
    }

    fn set_buyer_groups(&self, groups: Arc<dyn BuyerGroups>) -> Result<(), Error> {
        self.buyer_groups.set(groups).map_err(|_| twice("buyer_groups"))
    }

    fn add_tender(&self, tender: Arc<dyn Tender>) -> Result<(), Error> {
        let mut tenders =
            self.tenders.lock().map_err(|_| Error::internal("orders: tender list poisoned"))?;
        if tenders.iter().any(|existing| existing.kind() == tender.kind()) {
            return Err(twice(tender.kind()));
        }
        tenders.push(tender);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{BuyerProfile, SellerGroup, SplitLine};
    use autoxide_kernel::tenancy::TenantContext;

    struct Split;

    #[async_trait::async_trait]
    impl SellerSplit for Split {
        async fn split(
            &self,
            _tenant: &TenantContext,
            _lines: &[SplitLine],
        ) -> Result<Vec<SellerGroup>, Error> {
            Ok(Vec::new())
        }
    }

    struct Groups;

    #[async_trait::async_trait]
    impl BuyerGroups for Groups {
        async fn profile_of(
            &self,
            _tenant: &TenantContext,
            _buyer: uuid::Uuid,
        ) -> Result<BuyerProfile, Error> {
            Ok(BuyerProfile::default())
        }
    }

    #[test]
    fn moi_diem_cam_chi_cam_mot_lan() {
        let extensions = Extensions::default();
        extensions.set_seller_split(Arc::new(Split)).unwrap();
        assert!(extensions.set_seller_split(Arc::new(Split)).is_err());
        extensions.set_buyer_groups(Arc::new(Groups)).unwrap();
        assert!(extensions.set_buyer_groups(Arc::new(Groups)).is_err());
        assert!(extensions.buyer_groups().is_some());
    }
}
