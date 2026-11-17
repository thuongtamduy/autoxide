//! Test dòng tiền của `ledger` (cả SQLite và Postgres): ba chế độ sinh đúng bút toán, tạm giữ tới
//! hết hạn rồi nhả, hoàn trước khi nhả giảm phần phải trả, COD về thẳng người bán, chi trả thành
//! công / bị từ chối (ghi lại số dư) / đồng thời không vượt số dư, nhả đồng thời một lần, số dư
//! nhiều khoá, trạng thái niêm phong, bút toán theo chứng từ nguồn, tool MCP.

use std::sync::Arc;

use autoxide_kernel::event::Actor;
use autoxide_kernel::job::{EnqueueOptions, Job, JobQueue};
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::payment::{PaymentGateway, PayoutGateway};
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, OrgTree, TenantContext};
use autoxide_testkit::{
    FakePaymentGateway, FakePayoutGateway, FakePayoutOutcome, TestApp, TestDatabase, tool,
};
use axum::http::{Method, StatusCode};
use chrono::{Duration, Utc};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::{
    AccountKey, Collection, Counterparty, FlowEvent, Ledger, NewPayout, SaleAmounts, Settlement,
};
use crate::entity::{hold, payout};

const ALL: [&str; 7] = [
    "ledger.read",
    "ledger.post",
    "ledger.settings_manage",
    "ledger.payouts_create",
    "ledger.holds_release",
    "ledger.payments_create",
    "ledger.accounts_manage",
];

struct Setup {
    app: TestApp,
    payouts: Arc<FakePayoutGateway>,
    gateway: Arc<FakePaymentGateway>,
    /// Sàn (đặt chế độ), người bán dưới sàn, cửa hàng tự thu dưới gốc.
    market: OrgId,
    seller: OrgId,
    shop: OrgId,
}

async fn setup(test: &TestDatabase) -> Setup {
    let gateway = Arc::new(FakePaymentGateway::new());
    let payouts = Arc::new(FakePayoutGateway::new());
    let app = TestApp::builder(test)
        .user("boss", 1, &ALL)
        .contribute::<dyn PaymentGateway>(gateway.clone())
        .contribute::<dyn PayoutGateway>(payouts.clone())
        .module(crate::module())
        .build()
        .await
        .unwrap();
    let ids = || OrgId::from_uuid(Uuid::now_v7());
    let (market, seller, shop) = (ids(), ids(), ids());
    let write = app.database().begin_write().await.unwrap();
    let root = OrgTree::insert_root(&write, DEFAULT_TENANT).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, market, root, 8).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, seller, market, 8).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, shop, root, 8).await.unwrap();
    write.commit().await.unwrap();
    Setup { app, payouts, gateway, market, seller, shop }
}

fn ledger(app: &TestApp) -> Arc<dyn Ledger> {
    app.api::<dyn Ledger>().unwrap()
}

fn vnd(minor: i64) -> Money {
    Money::new(minor, Currency::VND)
}

fn sale(gross: i64, net: i64) -> SaleAmounts {
    SaleAmounts::new(vnd(gross), vnd(net), vnd(gross - net))
}

fn fake() -> Collection {
    Collection::Gateway { provider: "fake".to_owned() }
}

async fn call(app: &TestApp, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method, &format!("/api/v1/ledger{uri}")).token("boss");
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let body = if answer.body().is_empty() { Value::Null } else { answer.json() };
    (answer.status(), body)
}

async fn set_mode(app: &TestApp, org: OrgId, body: Value) {
    let (status, answer) = call(app, "PUT", &format!("/flow-settings/{org}"), Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
}

async fn settle(
    app: &TestApp,
    seller: OrgId,
    event: FlowEvent,
    source: &str,
    reference: &str,
) -> Result<crate::api::Settled, autoxide_kernel::error::Error> {
    let settlement = Settlement::new(seller, event, "orders", source, reference);
    ledger(app)
        .settle(&app.default_tenant(), Actor::system(), settlement)
        .await
        .map(|outcome| outcome.into_inner())
}

/// Số dư của tài khoản (mục đích, đối tượng) trong sổ của `book`.
async fn balance(app: &TestApp, book: OrgId, purpose: &str, party: Option<(&str, String)>) -> i64 {
    let mut key = AccountKey::new(book, purpose, Currency::VND);
    if let Some((kind, id)) = party {
        key = key.with_counterparty(Counterparty::new(kind, id));
    }
    ledger(app).balance(&app.default_tenant(), key, None).await.unwrap().minor()
}

fn seller_of(org: OrgId) -> Option<(&'static str, String)> {
    Some(("org", org.as_uuid().to_string()))
}

fn partner() -> Option<(&'static str, String)> {
    Some(("partner", "fake".to_owned()))
}

/// Tổng mọi dòng nợ = tổng mọi dòng có trong cả sổ (đọc qua bảng cân đối thử).
async fn book_balances(app: &TestApp, book: OrgId) {
    let (status, books) = call(app, "GET", "/books", None).await;
    assert_eq!(status, StatusCode::OK);
    let id = books["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["org_id"] == json!(book.as_uuid()))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let today = Utc::now().date_naive();
    let (status, trial) = call(
        app,
        "GET",
        &format!(
            "/trial-balance?book_id={id}&from={}&to={}",
            today - Duration::days(1),
            today + Duration::days(1)
        ),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{trial}");
    for total in trial["data"]["totals"].as_array().unwrap() {
        assert_eq!(total["debit"], total["credit"], "{trial}");
    }
}

/// Job lịch chạy ngay (không chờ lịch): đưa job của cả database vào hàng đợi rồi chạy.
#[derive(Debug, Serialize, Deserialize)]
struct ReleaseNow {}

impl Job for ReleaseNow {
    const KIND: &'static str = crate::flows::RELEASE_HOLDS;
}

#[derive(Debug, Serialize, Deserialize)]
struct CheckPayoutsNow {}

impl Job for CheckPayoutsNow {
    const KIND: &'static str = crate::payouts::CHECK_PAYOUTS;
}

async fn run_job<J: Job>(app: &TestApp, job: &J) {
    JobQueue::new("ledger")
        .enqueue_database(app.database().write(), job, EnqueueOptions::default())
        .await
        .unwrap();
    drain(app).await;
}

async fn drain(app: &TestApp) {
    let worker = app.modules().worker(4).unwrap().blocked_delay(std::time::Duration::ZERO);
    for _ in 0..5 {
        if worker.run_once().await.unwrap() == 0 {
            break;
        }
    }
}

async fn partner_mode(setup: &Setup, hold_days: i32) {
    set_mode(
        &setup.app,
        setup.market,
        json!({"mode": "partner_collect", "payment_provider": "fake", "payout_provider": "fake",
               "hold_days": hold_days}),
    )
    .await;
}

#[tokio::test]
async fn flow_self_collect_entries() {
    TestDatabase::run(async |test| {
        let setup = setup(test).await;
        let (app, shop) = (&setup.app, setup.shop);
        let settled = settle(
            app,
            shop,
            FlowEvent::Collected {
                amounts: sale(1_000_000, 909_091),
                via: fake(),
                operator_share: None,
            },
            "O-1",
            "O-1:collected",
        )
        .await
        .unwrap();
        assert_eq!(settled.mode, crate::api::FlowMode::SelfCollect);
        assert_eq!(settled.book_org, shop);
        assert!(settled.hold.is_none());
        let entry = settled.entry.unwrap();
        assert_eq!(entry.lines.len(), 3);
        assert_eq!(balance(app, shop, "clearing.gateway", partner()).await, 1_000_000);
        assert_eq!(balance(app, shop, "revenue.sales", None).await, 909_091);
        assert_eq!(balance(app, shop, "tax.vat_output", None).await, 90_909);
        // Phí cổng.
        settle(
            app,
            shop,
            FlowEvent::GatewayFeeCharged { provider: "fake".to_owned(), fee: vnd(1_100) },
            "O-1",
            "O-1:gateway-fee",
        )
        .await
        .unwrap();
        assert_eq!(balance(app, shop, "expense.gateway_fee", None).await, 1_100);
        assert_eq!(balance(app, shop, "clearing.gateway", partner()).await, 998_900);
        // Gửi lại cùng `reference`: không ghi thêm.
        let again = ledger(app)
            .settle(
                &app.default_tenant(),
                Actor::system(),
                Settlement::new(
                    shop,
                    FlowEvent::Collected {
                        amounts: sale(1_000_000, 909_091),
                        via: fake(),
                        operator_share: None,
                    },
                    "orders",
                    "O-1",
                    "O-1:collected",
                ),
            )
            .await
            .unwrap();
        assert!(matches!(again, autoxide_kernel::reference::Submitted::Replayed(_)));
        assert_eq!(balance(app, shop, "revenue.sales", None).await, 909_091);
        // Tự thu: sàn không trừ phí, không khấu trừ thay trên sổ của người bán.
        let fee = settle(
            app,
            shop,
            FlowEvent::FeeCharged { fee: vnd(50_000), vat: vnd(4_545) },
            "O-1",
            "O-1:fee",
        )
        .await;
        assert_eq!(fee.unwrap_err().code(), "ledger.flow_mode_unsupported");
        let withheld = settle(
            app,
            shop,
            FlowEvent::TaxWithheld { revenue: vnd(1_000_000), business_line: "goods".to_owned() },
            "O-1",
            "O-1:withheld",
        )
        .await;
        assert_eq!(withheld.unwrap_err().code(), "ledger.flow_mode_unsupported");
        // Hoàn: ngược lại doanh thu và thuế.
        settle(
            app,
            shop,
            FlowEvent::Refunded { amounts: sale(110_000, 100_000), via: fake() },
            "O-1",
            "O-1:refund",
        )
        .await
        .unwrap();
        assert_eq!(balance(app, shop, "revenue.sales", None).await, 809_091);
        assert_eq!(balance(app, shop, "tax.vat_output", None).await, 80_909);
        assert_eq!(balance(app, shop, "clearing.gateway", partner()).await, 888_900);
        book_balances(app, shop).await;
        // Bút toán theo chứng từ nguồn.
        let (status, listed) =
            call(app, "GET", "/entries?source_module=orders&source_id=O-1", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(listed["data"].as_array().unwrap().len(), 3, "{listed}");
        let (_, other) =
            call(app, "GET", "/entries?source_module=orders&source_id=O-2", None).await;
        assert!(other["data"].as_array().unwrap().is_empty());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn flow_partner_collect_hold_release_payout() {
    TestDatabase::run(async |test| {
        let setup = setup(test).await;
        let (app, market, seller) = (&setup.app, setup.market, setup.seller);
        partner_mode(&setup, 7).await;
        let (_, settings) = call(app, "GET", &format!("/flow-settings/{seller}"), None).await;
        assert_eq!(settings["data"]["mode"], "partner_collect");
        assert_eq!(settings["data"]["inherited"], true);
        let collected = settle(
            app,
            seller,
            FlowEvent::Collected {
                amounts: sale(1_000_000, 909_091),
                via: fake(),
                operator_share: None,
            },
            "O-1",
            "O-1:collected",
        )
        .await
        .unwrap();
        assert_eq!(collected.book_org, market);
        let held = collected.hold.unwrap();
        assert_eq!(held.amount.minor(), 1_000_000);
        assert_eq!(balance(app, market, "clearing.gateway", partner()).await, 1_000_000);
        assert_eq!(balance(app, market, "payable.seller_held", seller_of(seller)).await, 1_000_000);
        // Phí sàn 50.000 gồm GTGT 4.545 (tách riêng).
        let fee = settle(
            app,
            seller,
            FlowEvent::FeeCharged { fee: vnd(50_000), vat: vnd(4_545) },
            "O-1",
            "O-1:fee",
        )
        .await
        .unwrap();
        assert_eq!(fee.hold.unwrap().amount.minor(), 950_000);
        assert_eq!(balance(app, market, "revenue.platform_fee", None).await, 45_455);
        assert_eq!(balance(app, market, "tax.vat_output", None).await, 4_545);
        // Hoàn 100.000 trước khi hết hạn: giảm phần phải trả người bán.
        let refunded = settle(
            app,
            seller,
            FlowEvent::Refunded { amounts: sale(100_000, 90_909), via: fake() },
            "O-1",
            "O-1:refund-1",
        )
        .await
        .unwrap();
        assert_eq!(refunded.hold.unwrap().amount.minor(), 850_000);
        assert_eq!(balance(app, market, "payable.seller_held", seller_of(seller)).await, 850_000);
        assert_eq!(balance(app, market, "clearing.gateway", partner()).await, 900_000);
        // Hoàn quá số còn giữ: chặn, không ghi gì.
        let too_much = settle(
            app,
            seller,
            FlowEvent::Refunded { amounts: sale(900_000, 818_182), via: fake() },
            "O-1",
            "O-1:refund-2",
        )
        .await;
        assert_eq!(too_much.unwrap_err().code(), "ledger.insufficient_balance");
        // Chưa hết hạn: job không nhả. Chi trả lúc này: số dư khả dụng 0.
        run_job(app, &ReleaseNow {}).await;
        let status =
            ledger(app).hold_status(&app.default_tenant(), "orders", "O-1").await.unwrap().unwrap();
        assert_eq!(status.status, "held");
        let early = ledger(app)
            .create_payout(
                &app.default_tenant(),
                Actor::system(),
                NewPayout::new(seller, vnd(1), "Payout", "PO-0"),
            )
            .await;
        assert_eq!(early.unwrap_err().code(), "ledger.insufficient_balance");
        // Hết hạn đổi trả: job nhả phần còn giữ.
        hold::Entity::update_many()
            .col_expr(hold::Column::ReleaseAt, (Utc::now() - Duration::minutes(1)).into())
            .filter(hold::Column::Id.eq(held.id))
            .exec(app.database().write())
            .await
            .unwrap();
        run_job(app, &ReleaseNow {}).await;
        let status =
            ledger(app).hold_status(&app.default_tenant(), "orders", "O-1").await.unwrap().unwrap();
        assert_eq!(status.status, "released");
        assert!(status.release_entry_id.is_some());
        assert_eq!(balance(app, market, "payable.seller_held", seller_of(seller)).await, 0);
        assert_eq!(
            balance(app, market, "payable.seller_available", seller_of(seller)).await,
            850_000
        );
        // Phí sau khi nhả: trừ số dư khả dụng.
        settle(
            app,
            seller,
            FlowEvent::FeeCharged { fee: vnd(10_000), vat: vnd(909) },
            "O-1",
            "O-1:fee-2",
        )
        .await
        .unwrap();
        assert_eq!(
            balance(app, market, "payable.seller_available", seller_of(seller)).await,
            840_000
        );
        // Chi trả bị từ chối: số dư trở lại.
        setup.payouts.outcome(FakePayoutOutcome::Fail);
        let (status, failed) = call(
            app,
            "POST",
            "/payouts",
            Some(json!({"payee_org_id": seller, "amount": {"amount": "500000", "currency": "VND"},
                        "description": "Weekly payout", "reference": "PO-1"})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{failed}");
        assert_eq!(failed["data"]["status"], "failed");
        assert_eq!(
            balance(app, market, "payable.seller_available", seller_of(seller)).await,
            840_000
        );
        // Chi trả thành công: hết nghĩa vụ với người bán.
        setup.payouts.outcome(FakePayoutOutcome::Succeed);
        let (status, paid) = call(
            app,
            "POST",
            "/payouts",
            Some(json!({"payee_org_id": seller, "amount": {"amount": "840000", "currency": "VND"},
                        "description": "Weekly payout", "reference": "PO-2"})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{paid}");
        assert_eq!(paid["data"]["status"], "succeeded");
        // Gửi lại cùng reference: cùng lượt chi, đối tác không chi lần hai.
        let (status, again) = call(
            app,
            "POST",
            "/payouts",
            Some(json!({"payee_org_id": seller, "amount": {"amount": "840000", "currency": "VND"},
                        "description": "Weekly payout", "reference": "PO-2"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{again}");
        assert_eq!(again["data"]["id"], paid["data"]["id"]);
        assert_eq!(setup.payouts.count(), 2);
        assert_eq!(balance(app, market, "payable.seller_available", seller_of(seller)).await, 0);
        assert_eq!(balance(app, market, "payable.seller_held", seller_of(seller)).await, 0);
        // clearing = 1.000.000 − 100.000 hoàn − 840.000 chi.
        assert_eq!(balance(app, market, "clearing.gateway", partner()).await, 60_000);
        book_balances(app, market).await;
        let (_, listed) = call(app, "GET", "/payouts?status=failed", None).await;
        assert_eq!(listed["data"].as_array().unwrap().len(), 1, "{listed}");
        let (_, holds) = call(app, "GET", "/holds?status=released", None).await;
        assert_eq!(holds["data"].as_array().unwrap().len(), 1, "{holds}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn flow_split_at_gateway() {
    TestDatabase::run(async |test| {
        let setup = setup(test).await;
        let (app, market, seller) = (&setup.app, setup.market, setup.seller);
        // Cổng không chia được tiền: chế độ bị từ chối.
        setup.gateway.limited(true);
        let (status, refused) = call(
            app,
            "PUT",
            &format!("/flow-settings/{market}"),
            Some(json!({"mode": "split_at_gateway", "payment_provider": "fake"})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(refused["code"], "ledger.flow_mode_unsupported", "{refused}");
        setup.gateway.limited(false);
        set_mode(app, market, json!({"mode": "split_at_gateway", "payment_provider": "fake"}))
            .await;
        // Thiếu phần của sàn.
        let missing = settle(
            app,
            seller,
            FlowEvent::Collected {
                amounts: sale(1_000_000, 909_091),
                via: fake(),
                operator_share: None,
            },
            "O-1",
            "O-1:collected",
        )
        .await;
        assert_eq!(missing.unwrap_err().code(), "validation_failed");
        let collected = settle(
            app,
            seller,
            FlowEvent::Collected {
                amounts: sale(1_000_000, 909_091),
                via: fake(),
                operator_share: Some(vnd(65_000)),
            },
            "O-1",
            "O-1:collected",
        )
        .await
        .unwrap();
        assert!(collected.hold.is_none(), "chia tại cổng: không giữ");
        assert_eq!(balance(app, market, "clearing.gateway", partner()).await, 65_000);
        settle(
            app,
            seller,
            FlowEvent::FeeCharged { fee: vnd(50_000), vat: vnd(4_545) },
            "O-1",
            "O-1:fee",
        )
        .await
        .unwrap();
        assert_eq!(balance(app, market, "payable.seller_held", seller_of(seller)).await, 15_000);
        // COD không chia được ở cổng.
        let cod = settle(
            app,
            seller,
            FlowEvent::Collected {
                amounts: sale(100_000, 90_909),
                via: Collection::Cod { carrier: "ghn".to_owned() },
                operator_share: Some(vnd(5_000)),
            },
            "O-2",
            "O-2:collected",
        )
        .await;
        assert_eq!(cod.unwrap_err().code(), "ledger.flow_mode_unsupported");
        book_balances(app, market).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cod_ve_thang_nguoi_ban_phi_thanh_cong_no() {
    TestDatabase::run(async |test| {
        let setup = setup(test).await;
        let (app, market, seller) = (&setup.app, setup.market, setup.seller);
        set_mode(
            app,
            market,
            json!({"mode": "partner_collect", "payment_provider": "fake", "payout_provider": "fake",
                   "cod_remittance": "seller_direct"}),
        )
        .await;
        let cod = Collection::Cod { carrier: "ghn".to_owned() };
        let collected = settle(
            app,
            seller,
            FlowEvent::Collected { amounts: sale(300_000, 272_727), via: cod, operator_share: None },
            "O-1",
            "O-1:collected",
        )
        .await
        .unwrap();
        // Tiền COD không qua sàn: không bút toán, không tạm giữ.
        assert!(collected.entry.is_none() && collected.hold.is_none());
        settle(app, seller, FlowEvent::FeeCharged { fee: vnd(15_000), vat: vnd(1_364) }, "O-1", "O-1:fee")
            .await
            .unwrap();
        assert_eq!(balance(app, market, "receivable.seller", seller_of(seller)).await, 15_000);
        assert_eq!(balance(app, market, "clearing.carrier_cod", Some(("carrier", "ghn".to_owned()))).await, 0);
        // COD qua tài khoản thu hộ: ghi vào `clearing.carrier_cod`, có tạm giữ.
        set_mode(
            app,
            market,
            json!({"mode": "partner_collect", "payment_provider": "fake", "payout_provider": "fake"}),
        )
        .await;
        let collected = settle(
            app,
            seller,
            FlowEvent::Collected {
                amounts: sale(200_000, 181_818),
                via: Collection::Cod { carrier: "ghn".to_owned() },
                operator_share: None,
            },
            "O-2",
            "O-2:collected",
        )
        .await
        .unwrap();
        assert_eq!(collected.hold.unwrap().amount.minor(), 200_000);
        assert_eq!(balance(app, market, "clearing.carrier_cod", Some(("carrier", "ghn".to_owned()))).await, 200_000);
    })
    .await
    .unwrap();
}

/// Số dư khả dụng `amount` cho người bán: thu rồi nhả ngay (`hold_days` 0).
async fn available(setup: &Setup, amount: i64) {
    partner_mode(setup, 0).await;
    let settled = settle(
        &setup.app,
        setup.seller,
        FlowEvent::Collected { amounts: sale(amount, amount), via: fake(), operator_share: None },
        "O-1",
        "O-1:collected",
    )
    .await
    .unwrap();
    run_job(&setup.app, &ReleaseNow {}).await;
    let status = ledger(&setup.app)
        .hold_status(&setup.app.default_tenant(), "orders", "O-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status.status, "released", "{settled:?}");
}

#[tokio::test]
async fn chi_tra_dong_thoi_khong_vuot_so_du() {
    TestDatabase::run(async |test| {
        let setup = setup(test).await;
        available(&setup, 100_000).await;
        let tenant: TenantContext = setup.app.default_tenant();
        let api = ledger(&setup.app);
        let tasks = (0..8).map(|index| {
            let (api, tenant, seller) = (api.clone(), tenant.clone(), setup.seller);
            async move {
                api.create_payout(
                    &tenant,
                    Actor::system(),
                    NewPayout::new(seller, vnd(30_000), "Payout", format!("PO-{index}")),
                )
                .await
            }
        });
        let results = futures_util::future::join_all(tasks).await;
        let ok = results.iter().filter(|result| result.is_ok()).count();
        assert_eq!(ok, 3, "{results:?}");
        for result in results.iter().filter(|result| result.is_err()) {
            let code = result.as_ref().err().map(|error| error.code().to_owned());
            assert_eq!(code.as_deref(), Some("ledger.insufficient_balance"));
        }
        let left =
            balance(&setup.app, setup.market, "payable.seller_available", seller_of(setup.seller))
                .await;
        assert_eq!(left, 10_000);
        assert_eq!(setup.payouts.count(), 3);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nha_dong_thoi_mot_but_toan_va_chi_tra_cho_ket_qua() {
    TestDatabase::run(async |test| {
        let setup = setup(test).await;
        let app = &setup.app;
        partner_mode(&setup, 30).await;
        let held = settle(
            app,
            setup.seller,
            FlowEvent::Collected {
                amounts: sale(400_000, 363_636),
                via: fake(),
                operator_share: None,
            },
            "O-1",
            "O-1:collected",
        )
        .await
        .unwrap()
        .hold
        .unwrap();
        // Nhả sớm (người) sáu lượt cùng lúc: một bút toán.
        let path = format!("/holds/{}/release", held.id);
        let tasks = (0..6).map(|_| call(app, "POST", &path, None));
        let answers = futures_util::future::join_all(tasks).await;
        assert!(answers.iter().all(|(status, _)| *status == StatusCode::OK), "{answers:?}");
        let (_, entries) = call(app, "GET", "/entries?source_module=ledger", None).await;
        let releases = entries["data"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["source_kind"] == "hold_release")
            .count();
        assert_eq!(releases, 1, "{entries}");
        let available =
            balance(app, setup.market, "payable.seller_available", seller_of(setup.seller)).await;
        assert_eq!(available, 400_000);
        // Đối tác nhận lệnh, báo sau: `sent`, lịch hỏi lại → từ chối → số dư trở lại.
        setup.payouts.outcome(FakePayoutOutcome::Pending);
        let sent = ledger(app)
            .create_payout(
                &app.default_tenant(),
                Actor::system(),
                NewPayout::new(setup.seller, vnd(400_000), "Payout", "PO-1"),
            )
            .await
            .unwrap()
            .into_inner();
        assert_eq!(sent.status, "sent");
        setup.payouts.settle(sent.id, false);
        payout::Entity::update_many()
            .col_expr(payout::Column::UpdatedAt, (Utc::now() - Duration::minutes(5)).into())
            .filter(payout::Column::Id.eq(sent.id))
            .exec(app.database().write())
            .await
            .unwrap();
        run_job(app, &CheckPayoutsNow {}).await;
        let (_, found) = call(app, "GET", &format!("/payouts/{}", sent.id), None).await;
        assert_eq!(found["data"]["status"], "failed", "{found}");
        let available =
            balance(app, setup.market, "payable.seller_available", seller_of(setup.seller)).await;
        assert_eq!(available, 400_000);
        // Người bán tự thu: không chi trả qua sàn.
        let shop = ledger(app)
            .create_payout(
                &app.default_tenant(),
                Actor::system(),
                NewPayout::new(setup.shop, vnd(1), "Payout", "PO-2"),
            )
            .await;
        assert_eq!(shop.unwrap_err().code(), "ledger.flow_mode_unsupported");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn so_du_nhieu_khoa_niem_phong_va_tool_mcp() {
    TestDatabase::run(async |test| {
        let setup = setup(test).await;
        let app = &setup.app;
        partner_mode(&setup, 7).await;
        settle(
            app,
            setup.seller,
            FlowEvent::Collected {
                amounts: sale(500_000, 454_545),
                via: fake(),
                operator_share: None,
            },
            "O-1",
            "O-1:collected",
        )
        .await
        .unwrap();
        // Nhiều đối tượng một lượt (công nợ nhà cung cấp, người bán…): cùng thứ tự, chưa có = 0.
        let keys = vec![
            AccountKey::new(setup.market, "payable.seller_held", Currency::VND)
                .with_counterparty(Counterparty::new("org", setup.seller.as_uuid().to_string())),
            AccountKey::new(setup.market, "payable.supplier", Currency::VND)
                .with_counterparty(Counterparty::new("supplier", "S-1")),
            AccountKey::new(setup.market, "clearing.gateway", Currency::VND)
                .with_counterparty(Counterparty::new("partner", "fake")),
        ];
        let found = ledger(app).balances(&app.default_tenant(), &keys, None).await.unwrap();
        assert_eq!(
            found.iter().map(|money| money.minor()).collect::<Vec<_>>(),
            vec![500_000, 0, 500_000]
        );
        let today = Utc::now().date_naive();
        let at_day = ledger(app).balances(&app.default_tenant(), &keys, Some(today)).await.unwrap();
        assert_eq!(at_day, found);
        // Niêm phong: trước và sau job.
        let (_, books) = call(app, "GET", "/books", None).await;
        let book = books["data"][0]["id"].as_str().unwrap().to_owned();
        let (status, seal) = call(app, "GET", &format!("/books/{book}/seal-status"), None).await;
        assert_eq!(status, StatusCode::OK, "{seal}");
        assert_eq!(seal["data"]["total"], 1);
        assert_eq!(seal["data"]["sealed"], 0);
        assert!(seal["data"]["oldest_unsealed_at"].is_string());
        crate::service::LedgerService::seal_entries(app.database()).await.unwrap();
        let (_, seal) = call(app, "GET", &format!("/books/{book}/seal-status"), None).await;
        assert_eq!(seal["data"]["sealed"], 1, "{seal}");
        assert!(seal["data"]["oldest_unsealed_at"].is_null());
        let (_, entries) =
            call(app, "GET", "/entries?source_module=orders&source_id=O-1", None).await;
        assert_eq!(entries["data"][0]["sequence"], 1, "{entries}");
        assert!(entries["data"][0]["sealed_at"].is_string());
        // Tool MCP đi cùng service.
        let client = app.mcp("boss").await.unwrap();
        let answer = client
            .call_tool(tool("ledger.flow_settings", json!({"org_id": setup.seller})))
            .await
            .unwrap();
        assert_ne!(answer.is_error, Some(true), "{answer:?}");
        assert_eq!(answer.structured_content.unwrap()["data"]["mode"], "partner_collect");
        let answer =
            client.call_tool(tool("ledger.holds", json!({"status": "held"}))).await.unwrap();
        let held = answer.structured_content.unwrap();
        let hold_id = held["data"][0]["id"].as_str().unwrap().to_owned();
        let answer = client
            .call_tool(tool("ledger.release_hold", json!({"hold_id": hold_id})))
            .await
            .unwrap();
        assert_ne!(answer.is_error, Some(true), "{answer:?}");
        let answer = client
            .call_tool(tool(
                "ledger.create_payout",
                json!({"payee_org_id": setup.seller, "amount": "200000", "currency": "VND",
                       "description": "Payout", "reference": "PO-1"}),
            ))
            .await
            .unwrap();
        assert_ne!(answer.is_error, Some(true), "{answer:?}");
        assert_eq!(answer.structured_content.unwrap()["data"]["status"], "succeeded");
        let answer = client.call_tool(tool("ledger.payouts", json!({}))).await.unwrap();
        let listed = serde_json::to_string(&answer.structured_content).unwrap();
        assert!(listed.contains("PO-1"), "{listed}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn quyen_kiem_truoc_va_ngoai_pham_vi() {
    TestDatabase::run(async |test| {
        let setup = setup(test).await;
        let app = &setup.app;
        // Không có permission: 403 trước cả khi kiểm đầu vào.
        let answer = app
            .put(&format!("/api/v1/ledger/flow-settings/{}", setup.market))
            .json(json!({"mode": "nope"}))
            .send()
            .await
            .unwrap();
        assert_eq!(answer.status(), StatusCode::UNAUTHORIZED);
        let (status, invalid) = call(
            app,
            "PUT",
            &format!("/flow-settings/{}", setup.market),
            Some(json!({"mode": "nope", "hold_days": 999})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(invalid["errors"].as_array().unwrap().len(), 2, "{invalid}");
        let (status, _) =
            call(app, "POST", &format!("/holds/{}/release", Uuid::now_v7()), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = call(app, "GET", &format!("/payouts/{}", Uuid::now_v7()), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    })
    .await
    .unwrap();
}
