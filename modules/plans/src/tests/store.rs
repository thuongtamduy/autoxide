//! Mua trong ứng dụng (bước 69 P4): giao dịch giả hợp lệ cấp quyền dùng; gửi lại bằng chứng hay
//! thông báo không cấp trùng; hoàn tiền thu hồi quyền; chữ ký sai bị từ chối; hết hạn thì vòng đời
//! huỷ khi hết kỳ; khách không tự "tiếp tục" thành gói miễn phí. Chạy trên cả SQLite và Postgres.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::sync::Arc;

use autoxide_kernel::entitlement::TenantAccess;
use autoxide_kernel::store::{AppStore, PurchaseState, StorePurchase, StorePurchases};
use autoxide_testkit::{FakeStorePurchases, TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

use super::lifecycle::{call, tick};
use super::{Demo, handle, tenant};

const KEY: &str = "store-test-key";

async fn app(test: &TestDatabase) -> TestApp {
    let apple: Arc<dyn StorePurchases> = Arc::new(FakeStorePurchases::new(AppStore::Apple, KEY));
    TestApp::builder(test)
        .setting("AUTOXIDE_PLANS_CACHE_SECONDS", "0")
        .contribute::<dyn StorePurchases>(apple)
        .module(Demo::default())
        .module(crate::module())
        .tenant("acme")
        .tenant("beta")
        .user("ops", 1, &["plans.plans.manage", "plans.plans.read", "plans.subscriptions.read"])
        .user_in("acme", "owner", 2, &["plans.billing.read", "plans.billing.manage"])
        .user_in("beta", "other", 3, &["plans.billing.read", "plans.billing.manage"])
        .build()
        .await
        .unwrap()
}

fn store() -> FakeStorePurchases {
    FakeStorePurchases::new(AppStore::Apple, KEY)
}

fn bought(state: PurchaseState, days: i64, transaction: &str) -> StorePurchase {
    StorePurchase::new(
        AppStore::Apple,
        "pro_monthly",
        "orig-1",
        transaction,
        state,
        Some(Timestamp::now() + SignedDuration::from_hours(24 * days)),
    )
}

async fn ok(app: &TestApp, method: Method, uri: &str, who: &str, body: Option<Value>) -> Value {
    let (status, body) = call(app, method, uri, who, body).await;
    assert!(status.is_success(), "{uri}: {status} {body}");
    body
}

/// Gói `pro` (chính, đang bán) và nối sản phẩm `pro_monthly` của App Store với nó.
async fn setup(app: &TestApp) -> String {
    let body = json!({"code": "pro", "name": "Pro", "status": "active", "modules": ["demo"]});
    let created = ok(app, Method::POST, "/plans/plans", "ops", Some(body)).await;
    let plan = created["data"]["id"].as_str().unwrap().to_owned();
    let link = json!({"plan_id": plan});
    ok(app, Method::PUT, "/plans/store/products/app_store/pro_monthly", "ops", Some(link)).await;
    plan
}

async fn claim(app: &TestApp, who: &str, proof: &str) -> (StatusCode, Value) {
    let body = json!({"store": "app_store", "proof": proof});
    call(app, Method::POST, "/plans/store/purchases", who, Some(body)).await
}

async fn notify(app: &TestApp, store: &str, body: Vec<u8>) -> StatusCode {
    let answer = app
        .post(&format!("/api/v1/plans/store/notifications/{store}"))
        .header("content-type", "application/json")
        .bytes(body)
        .send()
        .await
        .unwrap();
    answer.status()
}

async fn events(app: &TestApp, name: &str) -> usize {
    use sea_orm::{ConnectionTrait, Statement};
    let control = app.registry().control().read();
    let sql = format!("SELECT id FROM plans_events WHERE name = '{name}'");
    let rows = control
        .query_all_raw(Statement::from_string(control.get_database_backend(), sql))
        .await
        .unwrap();
    rows.len()
}

#[tokio::test]
async fn giao_dich_hop_le_cap_quyen_dung_va_goi_lai_khong_cap_trung() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let plan = setup(&app).await;
        let proof = store().proof(&bought(PurchaseState::Active, 30, "t1"));

        let (status, sub) = claim(&app, "owner", &proof).await;
        assert_eq!(status, StatusCode::OK, "{sub}");
        assert_eq!(sub["data"]["plan_id"], plan.as_str());
        assert_eq!(sub["data"]["status"], "active");
        // Cửa hàng thu tiền: không giá của nhà vận hành, hết kỳ thì tự huỷ trừ khi cửa hàng gia hạn.
        assert!(sub["data"]["price_id"].is_null());
        assert_eq!(sub["data"]["cancel_at_period_end"], true);
        let acme = tenant(&app, "acme").await;
        let rights = handle(&app).snapshot(acme).await.unwrap();
        assert!(rights.has_module("demo") && rights.access == TenantAccess::Full);

        // Gửi lại cùng bằng chứng: không có thuê bao hay sự kiện mới.
        let (status, again) = claim(&app, "owner", &proof).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(again["data"]["id"], sub["data"]["id"]);
        assert_eq!(events(&app, "plans.subscription_created").await, 1);
        let mine = ok(&app, Method::GET, "/plans/store/purchases", "owner", None).await;
        assert_eq!(mine["data"].as_array().unwrap().len(), 1);
        assert_eq!(mine["data"][0]["state"], "active");

        // Cùng giao dịch ở khách khác: từ chối.
        let (status, taken) = claim(&app, "other", &proof).await;
        assert_eq!(
            (status, taken["code"].as_str()),
            (StatusCode::CONFLICT, Some("plans.store_purchase_taken"))
        );
        // Bằng chứng giả, hết hạn, sản phẩm chưa nối, cửa hàng chưa cài.
        let (status, _) = claim(&app, "other", "forged.0000").await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let expired = StorePurchase::new(
            AppStore::Apple,
            "pro_monthly",
            "orig-2",
            "t9",
            PurchaseState::Expired,
            None,
        );
        let (status, body) = claim(&app, "other", &store().proof(&expired)).await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::CONFLICT, Some("plans.store_purchase_inactive"))
        );
        let other = StorePurchase::new(
            AppStore::Apple,
            "gold",
            "orig-3",
            "t8",
            PurchaseState::Active,
            Some(Timestamp::now() + SignedDuration::from_hours(24 * 9)),
        );
        let (status, body) = claim(&app, "other", &store().proof(&other)).await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("plans.store_product_unknown"))
        );
        let google = json!({"store": "google_play", "proof": "x"});
        let (status, body) =
            call(&app, Method::POST, "/plans/store/purchases", "other", Some(google)).await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::SERVICE_UNAVAILABLE, Some("plans.store_unavailable"))
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn thong_bao_gia_han_gui_lai_khong_cap_trung_het_han_thi_huy() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        setup(&app).await;
        claim(&app, "owner", &store().proof(&bought(PurchaseState::Active, 30, "t1"))).await;

        let renewed = store().notification_body("n-1", &bought(PurchaseState::Active, 60, "t2"));
        assert_eq!(notify(&app, "app_store", renewed.clone()).await, StatusCode::NO_CONTENT);
        assert_eq!(notify(&app, "app_store", renewed).await, StatusCode::NO_CONTENT);
        assert_eq!(events(&app, "plans.subscription_plan_changed").await, 1);
        let sub = ok(&app, Method::GET, "/plans/subscription", "owner", None).await;
        let end =
            Timestamp::parse_rfc3339(sub["data"]["current_period_end"].as_str().unwrap()).unwrap();
        assert!(end > Timestamp::now() + SignedDuration::from_hours(24 * 59));

        // Khách không tự "tiếp tục" / đổi ở đây: phải làm trong cửa hàng.
        let (status, body) =
            call(&app, Method::POST, "/plans/subscription/resume", "owner", None).await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::CONFLICT, Some("plans.store_billed"))
        );
        // Tool MCP cùng permission cũng không đổi được (luật kiến trúc 8; rà soát bảo mật 2026-11).
        let client = app.mcp("owner").await.unwrap();
        let answer = client
            .call_tool(autoxide_testkit::tool(
                "plans.change_subscription",
                json!({"plan_id": uuid::Uuid::now_v7(), "reference": "r-1"}),
            ))
            .await
            .unwrap();
        assert_eq!(answer.is_error, Some(true));
        let text = serde_json::to_string(&answer.structured_content).unwrap();
        assert!(text.contains("plans.store_billed"), "{text}");

        // Hết hạn, không gia hạn: vòng đời huỷ khi hết kỳ (không thành gói miễn phí).
        let gone = StorePurchase::new(
            AppStore::Apple,
            "pro_monthly",
            "orig-1",
            "t2",
            PurchaseState::Expired,
            Some(Timestamp::now() - SignedDuration::from_hours(24)),
        );
        assert_eq!(
            notify(&app, "app_store", store().notification_body("n-2", &gone)).await,
            StatusCode::NO_CONTENT
        );
        tick(&app).await;
        let sub = ok(&app, Method::GET, "/plans/subscription", "owner", None).await;
        assert_eq!(sub["data"]["status"], "canceled");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hoan_tien_thu_hoi_quyen_va_chu_ky_sai_bi_tu_choi() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        setup(&app).await;
        claim(&app, "owner", &store().proof(&bought(PurchaseState::Active, 30, "t1"))).await;

        // Chữ ký sai, cửa hàng lạ, cửa hàng chưa cài: không đổi gì.
        let forged = FakeStorePurchases::new(AppStore::Apple, "wrong-key")
            .notification_body("n-x", &bought(PurchaseState::Refunded, 30, "t1"));
        assert_eq!(notify(&app, "app_store", forged).await, StatusCode::UNAUTHORIZED);
        assert_eq!(notify(&app, "nowhere", b"{}".to_vec()).await, StatusCode::NOT_FOUND);
        assert_eq!(
            notify(&app, "google_play", b"{}".to_vec()).await,
            StatusCode::SERVICE_UNAVAILABLE
        );
        let sub = ok(&app, Method::GET, "/plans/subscription", "owner", None).await;
        assert_eq!(sub["data"]["status"], "active");

        // Hoàn tiền: thôi quyền dùng ngay (chỉ đọc), kể cả khi thông báo đến lại.
        let refund = store().notification_body("n-3", &bought(PurchaseState::Refunded, 30, "t1"));
        assert_eq!(notify(&app, "app_store", refund.clone()).await, StatusCode::NO_CONTENT);
        assert_eq!(notify(&app, "app_store", refund).await, StatusCode::NO_CONTENT);
        let sub = ok(&app, Method::GET, "/plans/subscription", "owner", None).await;
        assert_eq!(sub["data"]["status"], "canceled");
        let acme = tenant(&app, "acme").await;
        assert_eq!(handle(&app).snapshot(acme).await.unwrap().access, TenantAccess::ReadOnly);
        // Không "tiếp tục" miễn phí được.
        let (status, body) =
            call(&app, Method::POST, "/plans/subscription/resume", "owner", None).await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::CONFLICT, Some("plans.store_billed"))
        );
        // Thông báo về giao dịch không gắn khách nào: bỏ qua.
        let stray = StorePurchase::new(
            AppStore::Apple,
            "pro_monthly",
            "orig-9",
            "t1",
            PurchaseState::Active,
            Some(Timestamp::now() + SignedDuration::from_hours(24 * 3)),
        );
        assert_eq!(
            notify(&app, "app_store", store().notification_body("n-4", &stray)).await,
            StatusCode::NO_CONTENT
        );
    })
    .await
    .unwrap();
}
