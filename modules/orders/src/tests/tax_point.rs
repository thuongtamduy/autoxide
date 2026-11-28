//! Ngày thuế của đơn (LG-2): theo múi giờ của vùng thuế người bán, không theo UTC — ranh giới 23:59:59 /
//! 00:00:00 giờ Việt Nam (16:59:59Z / 17:00:00Z); vùng không khai múi giờ thì đi đường đủ của engine.

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{OrgId, TenantContext};
use autoxide_kernel::time::{Date, Timestamp};
use autoxide_tax::api::{
    InvoiceRequirements, TaxEngine, TaxProfile, TaxQuote, TaxQuoteRequest, WithholdingQuote,
    WithholdingRequest,
};
use autoxide_testkit::{TestApp, TestDatabase};

/// Engine giả: vùng `VN` có múi giờ Việt Nam; vùng khác không khai, đường đủ trả ngày cố định.
struct Engine;

#[async_trait::async_trait]
impl TaxEngine for Engine {
    async fn quote(&self, _: &TenantContext, _: TaxQuoteRequest) -> Result<TaxQuote, Error> {
        Err(Error::internal("not used by this test"))
    }
    async fn withholding(
        &self,
        _: &TenantContext,
        _: WithholdingRequest,
    ) -> Result<WithholdingQuote, Error> {
        Err(Error::internal("not used by this test"))
    }
    async fn profile(&self, _: &TenantContext, _: OrgId) -> Result<Option<TaxProfile>, Error> {
        Err(Error::internal("not used by this test"))
    }
    async fn invoice_requirements(
        &self,
        _: &TenantContext,
        _: OrgId,
        _: Date,
    ) -> Result<InvoiceRequirements, Error> {
        Err(Error::internal("not used by this test"))
    }
    async fn tax_date(&self, _: &TenantContext, _: OrgId, _: Timestamp) -> Result<Date, Error> {
        Ok(Date::new(2000, 1, 1).unwrap())
    }
    async fn tax_date_in(&self, jurisdiction: &str, at: Timestamp) -> Result<Option<Date>, Error> {
        let zone = autoxide_kernel::time::zone("Asia/Ho_Chi_Minh").unwrap();
        Ok((jurisdiction == "VN").then(|| at.date_in(&zone)))
    }
}

#[tokio::test]
async fn ngay_thue_cua_don_theo_gio_viet_nam_o_ranh_gioi_nua_dem() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test).build().await.unwrap();
        let tenant = app.default_tenant();
        let org = OrgId::from_uuid(uuid::Uuid::now_v7());
        let at = |text: &str| Timestamp::parse_rfc3339(text).unwrap();
        let point = async |jurisdiction: &str, instant: &str| {
            crate::pricing::tax_point(&Engine, &tenant, jurisdiction, org, at(instant))
                .await
                .unwrap()
        };
        // 23:59:59 giờ Việt Nam ngày 31/12/2026: còn năm 2026.
        assert_eq!(point("VN", "2026-12-31T16:59:59Z").await, Date::new(2026, 12, 31).unwrap());
        // 00:00:00 giờ Việt Nam ngày 01/01/2027 (vẫn 31/12 theo UTC): năm 2027.
        assert_eq!(point("VN", "2026-12-31T17:00:00Z").await, Date::new(2027, 1, 1).unwrap());
        // Vùng không khai múi giờ: đường đủ của engine (múi giờ của nút).
        assert_eq!(point("XX", "2026-12-31T17:00:00Z").await, Date::new(2000, 1, 1).unwrap());
    })
    .await
    .unwrap();
}

/// Engine giả theo giờ Việt Nam cho mọi người bán (như vùng thuế `vn`).
struct Vietnam;

#[async_trait::async_trait]
impl TaxEngine for Vietnam {
    async fn quote(&self, _: &TenantContext, _: TaxQuoteRequest) -> Result<TaxQuote, Error> {
        Err(Error::internal("not used by this test"))
    }
    async fn withholding(
        &self,
        _: &TenantContext,
        _: WithholdingRequest,
    ) -> Result<WithholdingQuote, Error> {
        Err(Error::internal("not used by this test"))
    }
    async fn profile(&self, _: &TenantContext, _: OrgId) -> Result<Option<TaxProfile>, Error> {
        Err(Error::internal("not used by this test"))
    }
    async fn invoice_requirements(
        &self,
        _: &TenantContext,
        _: OrgId,
        _: Date,
    ) -> Result<InvoiceRequirements, Error> {
        Err(Error::internal("not used by this test"))
    }
    async fn tax_date(&self, _: &TenantContext, _: OrgId, at: Timestamp) -> Result<Date, Error> {
        Ok(at.date_in(&autoxide_kernel::time::zone("Asia/Ho_Chi_Minh").unwrap()))
    }
}

/// Năm trong số đơn, đơn tại quầy, phiếu trả, phiếu bảo hành (#575 N4): theo ngày của người bán như ngày thuế
/// (LG-2) — 23:59:59 giờ Việt Nam ngày 31/12/2026 còn năm 2026, 00:00:00 ngày 01/01/2027 đã năm 2027 dù UTC vẫn
/// 31/12. Không cài `tax` lẫn `organizations`: năm UTC như trước.
#[tokio::test]
async fn nam_cua_so_chung_tu_theo_gio_nguoi_ban_o_giao_thua() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test).build().await.unwrap();
        let tenant = app.default_tenant();
        let org = OrgId::from_uuid(uuid::Uuid::now_v7());
        let at = |text: &str| Timestamp::parse_rfc3339(text).unwrap();
        let year = async |engine: Option<&dyn TaxEngine>, instant: &str| {
            crate::local_year::local_year(engine, None, &tenant, org, at(instant)).await.unwrap()
        };
        assert_eq!(year(Some(&Vietnam), "2026-12-31T16:59:59Z").await, 2026);
        assert_eq!(year(Some(&Vietnam), "2026-12-31T17:00:00Z").await, 2027);
        assert_eq!(year(None, "2026-12-31T17:00:00Z").await, 2026);
        assert_eq!(year(None, "2027-01-01T00:00:00Z").await, 2027);
    })
    .await
    .unwrap();
}
