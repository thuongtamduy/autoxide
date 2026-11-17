//! Dòng tiền của `ledger` cùng `tax` trên chương trình thật (cả SQLite và Postgres): khấu trừ thay
//! theo NĐ 117/2025 (cần đối chiếu) chỉ cho hộ / cá nhân kinh doanh, tỉ lệ đọc từ gói thuế "vn";
//! doanh nghiệp tự kê khai thì không sinh bút toán; mã số thuế 12 số (TT 86/2024 — cần đối chiếu)
//! được nhận ở hồ sơ thuế.

use std::sync::Arc;

use autoxide_kernel::event::Actor;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::payment::{PaymentGateway, PayoutGateway};
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, OrgTree, root_of};
use autoxide_ledger::api::{
    AccountKey, Collection, Counterparty, FlowEvent, Ledger, SaleAmounts, Settlement,
};
use autoxide_testkit::{FakePaymentGateway, FakePayoutGateway, TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use uuid::Uuid;

const PERMISSIONS: [&str; 5] = [
    "tax.read",
    "tax.packs_manage",
    "tax.profiles_manage",
    "ledger.read",
    "ledger.settings_manage",
];

async fn call(app: &TestApp, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method, &format!("/api/v1{uri}")).token("ops");
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let body = if answer.body().is_empty() { Value::Null } else { answer.json() };
    (answer.status(), body)
}

fn vnd(minor: i64) -> Money {
    Money::new(minor, Currency::VND)
}

async fn held(app: &TestApp, market: OrgId, seller: OrgId) -> i64 {
    let key = AccountKey::new(market, "payable.seller_held", Currency::VND)
        .with_counterparty(Counterparty::new("org", seller.as_uuid().to_string()));
    let ledger = app.api::<dyn Ledger>().unwrap();
    ledger.balance(&app.default_tenant(), key, None).await.unwrap().minor()
}

#[tokio::test]
async fn khau_tru_thay_chi_cho_ho_kinh_doanh() {
    TestDatabase::run(async |test| {
        let mut builder = TestApp::builder(test)
            .user("ops", 1, &PERMISSIONS)
            .contribute::<dyn PaymentGateway>(Arc::new(FakePaymentGateway::new()))
            .contribute::<dyn PayoutGateway>(Arc::new(FakePayoutGateway::new()));
        for module in crate::modules::all() {
            builder = builder.boxed_module(module);
        }
        let app = builder.build().await.unwrap();
        let ids = || OrgId::from_uuid(Uuid::now_v7());
        let (market, household, company) = (ids(), ids(), ids());
        let write = app.database().begin_write().await.unwrap();
        let _ = OrgTree::insert_root(&write, DEFAULT_TENANT).await;
        OrgTree::insert(&write, DEFAULT_TENANT, market, root_of(DEFAULT_TENANT), 8).await.unwrap();
        OrgTree::insert(&write, DEFAULT_TENANT, household, market, 8).await.unwrap();
        OrgTree::insert(&write, DEFAULT_TENANT, company, market, 8).await.unwrap();
        write.commit().await.unwrap();
        let pack: Value =
            serde_json::from_str(include_str!("../../../../modules/tax/packs/vn.json")).unwrap();
        let (status, answer) = call(&app, "POST", "/tax/packs", Some(json!({"pack": pack}))).await;
        assert_eq!(status, StatusCode::OK, "{answer}");
        // Hộ kinh doanh dùng số định danh cá nhân 12 số làm mã số thuế.
        let profile = json!({"jurisdiction": "VN", "tax_code": "001234567890", "regime": "household",
                             "business_lines": ["goods"]});
        let (status, answer) =
            call(&app, "PUT", &format!("/tax/profiles/{household}"), Some(profile)).await;
        assert_eq!(status, StatusCode::OK, "{answer}");
        let profile = json!({"jurisdiction": "VN", "tax_code": "0101234567", "regime": "vat_credit",
                             "business_lines": ["goods"]});
        let (status, answer) =
            call(&app, "PUT", &format!("/tax/profiles/{company}"), Some(profile)).await;
        assert_eq!(status, StatusCode::OK, "{answer}");
        let (status, answer) = call(
            &app,
            "PUT",
            &format!("/ledger/flow-settings/{market}"),
            Some(json!({"mode": "partner_collect", "payment_provider": "fake", "payout_provider": "fake"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{answer}");
        let ledger = app.api::<dyn Ledger>().unwrap();
        let tenant = app.default_tenant();
        let sale = SaleAmounts::new(vnd(1_000_000), vnd(909_091), vnd(90_909));
        for (seller, order) in [(household, "O-1"), (company, "O-2")] {
            let collected = FlowEvent::Collected {
                amounts: sale,
                via: Collection::Gateway { provider: "fake".to_owned() },
                operator_share: None,
            };
            let settlement =
                Settlement::new(seller, collected, "orders", order, format!("{order}:collected"));
            ledger.settle(&tenant, Actor::system(), settlement).await.unwrap();
        }
        let withhold = |seller: OrgId, order: &str| {
            Settlement::new(
                seller,
                FlowEvent::TaxWithheld { revenue: vnd(1_000_000), business_line: "goods".to_owned() },
                "orders",
                order,
                format!("{order}:withheld"),
            )
        };
        // Hộ kinh doanh, hàng hoá: 1% GTGT + 0,5% TNCN theo gói "vn".
        let done = ledger.settle(&tenant, Actor::system(), withhold(household, "O-1")).await.unwrap();
        let done = done.into_inner();
        assert_eq!(done.entry.as_ref().unwrap().lines.len(), 3);
        assert_eq!(done.hold.unwrap().amount.minor(), 985_000);
        assert_eq!(held(&app, market, household).await, 985_000);
        let vat = AccountKey::new(market, "tax.withheld_vat", Currency::VND);
        let pit = AccountKey::new(market, "tax.withheld_pit", Currency::VND);
        let found = ledger.balances(&tenant, &[vat, pit], None).await.unwrap();
        assert_eq!((found[0].minor(), found[1].minor()), (10_000, 5_000));
        // Doanh nghiệp tự kê khai: không khấu trừ, không bút toán.
        let skipped = ledger.settle(&tenant, Actor::system(), withhold(company, "O-2")).await.unwrap();
        assert!(skipped.into_inner().entry.is_none());
        assert_eq!(held(&app, market, company).await, 1_000_000);
    })
    .await
    .unwrap();
}
