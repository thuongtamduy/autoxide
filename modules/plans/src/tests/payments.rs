//! Tiền thuê bao về qua `ledger.payment_succeeded`: chỉ coi kỳ là đã trả khi số thực thu đủ số phải
//! trả. `ledger` ghi nhận cả khoản thu thiếu (đối soát `accept_received`), nên event có thể mang số
//! nhỏ hơn số của kỳ.

use std::sync::Arc;

use autoxide_kernel::money::Money;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::Method;
use serde_json::{Value, json};
use uuid::Uuid;

use super::fake_ledger::{FakeLedgerModule, succeed};
use super::lifecycle::{ok, priced, sql, tick};

async fn app(test: &TestDatabase, ledger: FakeLedgerModule) -> TestApp {
    TestApp::builder(test)
        .setting("AUTOXIDE_PLANS_CACHE_SECONDS", "0")
        .module(ledger)
        .module(crate::module())
        .tenant("acme")
        .user("ops", 1, &["plans.plans.manage", "plans.subscriptions.manage"])
        .user_in("acme", "owner", 2, &["plans.billing.read", "plans.billing.manage"])
        .build()
        .await
        .unwrap()
}

async fn renewal(app: &TestApp) -> Value {
    let periods = ok(app, Method::GET, "/plans/billing-periods", "owner", None).await;
    periods["data"].as_array().unwrap().iter().find(|p| p["kind"] == "renewal").unwrap().clone()
}

async fn deliver(app: &TestApp) {
    let dispatcher = app.modules().dispatcher().unwrap();
    while dispatcher.run_once().await.unwrap() > 0 {}
}

#[tokio::test]
async fn thu_thieu_khong_danh_dau_ky_da_tra() {
    TestDatabase::run(async |test| {
        let ledger = FakeLedgerModule::default();
        let outbox = Arc::clone(&ledger.outbox);
        let app = app(test, ledger).await;
        let pro = priced(&app, "pro", 199_000, 0).await;
        let tenant = app.tenant("acme").await.unwrap().id();
        let body = json!({"plan_id": pro.0, "price_id": pro.1, "trial": false});
        let uri = format!("/plans/tenants/{tenant}/subscription");
        ok(&app, Method::PUT, &uri, "ops", Some(body)).await;
        sql(&app, "UPDATE plans_subscription SET current_period_end = '2000-01-01T00:00:00Z'")
            .await;
        tick(&app).await;
        let period = renewal(&app).await;
        assert_eq!(period["status"], "open");
        let id: Uuid = period["id"].as_str().unwrap().parse().unwrap();
        let operator = app.default_tenant();

        // Thu thiếu (đối soát chấp nhận số thực nhận): kỳ vẫn mở.
        succeed(&outbox, &operator, id, Money::parse("150000", "VND").unwrap()).await;
        deliver(&app).await;
        assert_eq!(renewal(&app).await["status"], "open");
        // Khác loại tiền: không coi là đủ.
        succeed(&outbox, &operator, id, Money::parse("199000", "USD").unwrap()).await;
        deliver(&app).await;
        assert_eq!(renewal(&app).await["status"], "open");

        // Thu đủ: đã trả.
        succeed(&outbox, &operator, id, Money::parse("199000", "VND").unwrap()).await;
        deliver(&app).await;
        let paid = renewal(&app).await;
        assert_eq!(paid["status"], "paid", "{paid}");
    })
    .await
    .unwrap();
}
