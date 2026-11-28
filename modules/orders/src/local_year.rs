//! Năm trong số chứng từ của `orders` — đơn (`2027-000001`), bán tại quầy, trả hàng, phiếu bảo hành (`BH2027-…`) —
//! theo ngày của người bán, không theo ngày UTC (#575 N4). Cùng một ngày với ngày thuế của LG-2
//! (D-20261126-tax-date-time-zone): múi giờ của vùng thuế trong hồ sơ thuế (Việt Nam: `Asia/Ho_Chi_Minh`), rồi múi
//! giờ của nút (`organizations.timezone`), rồi UTC. Không thì đơn đặt từ 00:00 tới 07:00 ngày 01/01 giờ Việt Nam
//! mang số của năm trước, lẫn chuỗi số của hai năm.

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{OrgId, TenantContext};
use autoxide_kernel::time::Timestamp;
use autoxide_organizations::api::OrgDirectory;
use autoxide_tax::api::TaxEngine;

use crate::service::OrdersService;

impl OrdersService {
    /// Năm của số chứng từ lập lúc `at` ở nút bán `org`. Đọc database của module khác (hồ sơ thuế, cài đặt nút):
    /// gọi TRƯỚC transaction ghi.
    pub(crate) async fn local_year(
        &self,
        tenant: &TenantContext,
        org: OrgId,
        at: Timestamp,
    ) -> Result<i16, Error> {
        local_year(self.sources.tax.as_deref(), self.orgs.as_deref(), tenant, org, at).await
    }
}

/// Như [`OrdersService::local_year`], với engine thuế và danh bạ nút có thể chưa cài.
pub(crate) async fn local_year(
    tax: Option<&dyn TaxEngine>,
    orgs: Option<&dyn OrgDirectory>,
    tenant: &TenantContext,
    org: OrgId,
    at: Timestamp,
) -> Result<i16, Error> {
    if let Some(engine) = tax {
        // `tax_date` đã gồm cả múi giờ của nút và UTC khi vùng thuế không khai múi giờ.
        return Ok(engine.tax_date(tenant, org, at).await?.year());
    }
    if let Some(orgs) = orgs {
        let zone = orgs
            .setting(tenant, org, "organizations.timezone")
            .await?
            .and_then(|found| found.value)
            .and_then(|value| value.as_str().and_then(autoxide_kernel::time::zone));
        if let Some(zone) = zone {
            return Ok(at.date_in(&zone).year());
        }
    }
    Ok(at.date_utc().year())
}
