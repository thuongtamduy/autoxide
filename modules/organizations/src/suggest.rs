//! Nguồn gợi ý tìm của `organizations` (D-201) cho `GET /api/v1/catalog/public/search/suggest`:
//! cửa hàng công khai của khách, đúng tập `GET /api/v1/organizations/public/stores` (hồ sơ công
//! khai, nút đang hoạt động). Số cửa hàng của một khách nhỏ nên lọc trong ứng dụng, như danh mục
//! của `catalog`. Hai tầng (D-202): tên khớp đúng dấu trước, khớp không dấu sau.

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::search;
use autoxide_kernel::suggest::{SuggestSource, Suggestion};
use autoxide_kernel::tenancy::{Site, TenantContext};

use crate::service::OrgService;

/// Cửa hàng công khai. `context` là địa chỉ ngắn (phường / xã, tỉnh); website mở cửa hàng theo `id`.
pub(crate) struct Stores(pub(crate) Arc<OrgService>);

#[async_trait::async_trait]
impl SuggestSource for Stores {
    fn kind(&self) -> &'static str {
        "organizations.store"
    }

    async fn suggest(
        &self,
        tenant: &TenantContext,
        _site: &Site,
        _locale: &str,
        q: &str,
        limit: u16,
    ) -> Result<Vec<Suggestion>, Error> {
        let stores = self.0.public_stores(tenant).await?;
        let wanted_exact = search::exact_key(q);
        let wanted = search::fold(q);
        let mut exact = Vec::new();
        let mut loose = Vec::new();
        for store in stores {
            let place = store
                .address
                .as_ref()
                .map(|address| format!("{}, {}", address.commune_name, address.province_name))
                .unwrap_or_default();
            // Tìm theo tên và địa chỉ: người mua hay gõ tên phường / quận để tìm cửa hàng gần.
            let haystack = format!("{} {place}", store.display_name);
            let is_exact = search::exact_key(&haystack).contains(&wanted_exact);
            if !is_exact && !search::fold(&haystack).contains(&wanted) {
                continue;
            }
            let item = Suggestion::new("organizations.store", store.display_name, is_exact)
                .id(store.unit_id)
                .context(place);
            if is_exact { exact.push(item) } else { loose.push(item) }
        }
        exact.extend(loose);
        exact.truncate(usize::from(limit));
        Ok(exact)
    }
}
