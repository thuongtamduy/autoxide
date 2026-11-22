//! Thu hộ khi giao (bước 44 mục 5) với module `ledger` thật: giao xong ghi phải thu hãng một lần,
//! bảng kê khớp ghi sổ cái, lệch tiền / mã lạ sinh chênh lệch, nhập lại bảng kê không nhân đôi, quá
//! hạn chưa đối soát, giải quyết chênh lệch có mã lý do. Cả SQLite và Postgres.

use autoxide_kernel::access::Scope;
use autoxide_kernel::money::Currency;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, OrgTree, root_of};
use autoxide_kernel::time::Timestamp;
use autoxide_ledger::api::{AccountKey, Counterparty, Ledger};
use jiff::SignedDuration;

use super::fake_ledger::FakeLedger;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use super::*;
use crate::api::CarrierRegistry;
use crate::api::fake::FakeCarrier;

const ALL: [&str; 6] = [
    "shipping.settings.manage",
    "shipping.read",
    "shipping.carriers.manage",
    "shipping.shipments.read",
    "shipping.shipments.manage",
    "shipping.cod.manage",
];

fn vnd(amount: i64) -> Value {
    json!({"amount": amount.to_string(), "currency": "VND"})
}

/// App có `ledger` thật, cửa hàng `store`, tài khoản hãng giả và phương thức `carrier`. Trả (app,
/// cửa hàng, ID tài khoản hãng, ID phương thức).
async fn cod_shop(
    test: &TestDatabase,
    settings: &[(&str, &str)],
) -> (TestApp, OrgId, String, String) {
    let store = OrgId::from_uuid(Uuid::now_v7());
    let mut builder = TestApp::builder(test)
        .member("boss", 1, root_of(DEFAULT_TENANT), &ALL.map(|name| (name, Scope::Tenant)))
        .member("auditor", 2, root_of(DEFAULT_TENANT), &[("shipping.cod.read", Scope::Tenant)])
        .provide::<dyn AdministrativeUnits>(Arc::new(FakeUnits))
        .provide::<dyn Ledger>(Arc::new(FakeLedger::default()))
        .module(crate::module());
    for (name, value) in settings {
        builder = builder.setting(name, value);
    }
    let app = builder.build().await.unwrap();
    let write = app.database().begin_write().await.unwrap();
    let root = OrgTree::insert_root(&write, DEFAULT_TENANT).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, store, root, 8).await.unwrap();
    write.commit().await.unwrap();
    app.api::<dyn CarrierRegistry>().unwrap().register(Arc::new(FakeCarrier::new())).unwrap();
    let body = json!({"org_id": store.as_uuid(), "carrier_code": "fake", "name": "Fake",
                      "credentials": {"token": "t", "webhook_secret": "s"}});
    let (status, account) = call(&app, "POST", "/carrier-accounts", "boss", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{account}");
    let account = account["data"]["id"].as_str().unwrap().to_owned();
    let body = json!({"org_id": store.as_uuid(), "code": "fast", "kind": "carrier", "names": {"vi": "Nhanh"},
                      "currency": "VND", "carrier_account_id": account});
    let (status, method) = call(&app, "POST", "/methods", "boss", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{method}");
    (app, store, account, method["data"]["id"].as_str().unwrap().to_owned())
}

/// Vận đơn COD 520 000đ đã đặt hãng; trả (ID, mã vận đơn).
async fn booked(app: &TestApp, store: OrgId, method: &str, key: &str) -> (String, String) {
    let body = json!({
        "order_id": Uuid::now_v7(), "order_number": "DH1", "request_key": key,
        "org_id": store.as_uuid(), "method_id": method,
        "recipient_name": "An", "recipient_phone": "0912345678",
        "address": {"country": "VN", "province_code": "01", "commune_code": "00004", "line": "1"},
        "items": [{"order_line_key": "l1", "variant_id": Uuid::now_v7(), "quantity": 1, "weight_grams": 500}],
        "declared_value": vnd(500_000), "cod_amount": vnd(520_000),
    });
    let (status, created) = call(app, "POST", "/shipments", "boss", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["data"]["id"].as_str().unwrap().to_owned();
    let (status, booked) = call(app, "POST", &format!("/shipments/{id}/book"), "boss", None).await;
    assert_eq!(status, StatusCode::OK, "{booked}");
    (id, booked["data"]["tracking_number"].as_str().unwrap().to_owned())
}

async fn deliver(app: &TestApp, id: &str) {
    for step in ["picked_up", "in_transit", "delivered"] {
        let (status, body) = call(
            app,
            "POST",
            &format!("/shipments/{id}/status"),
            "boss",
            Some(json!({"status": step})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    drain(app).await;
}

async fn carrier_balance(app: &TestApp, store: OrgId, account: &str) -> i64 {
    let key = AccountKey::new(store, "clearing.carrier_cod", Currency::VND)
        .with_counterparty(Counterparty::new("shipping.carrier_account", account));
    balance(app, key).await
}

async fn balance(app: &TestApp, key: AccountKey) -> i64 {
    let ledger = app.api::<dyn Ledger>().unwrap();
    ledger.balance(&app.default_tenant(), key, None).await.unwrap().minor()
}

async fn import(app: &TestApp, account: &str, reference: &str, csv: &str) -> (StatusCode, Value) {
    let body = json!({"carrier_account_id": account, "reference": reference,
                      "period_from": "2026-11-01", "period_to": "2026-11-30", "csv": csv});
    call(app, "POST", "/cod/statements", "boss", Some(body)).await
}

#[tokio::test]
async fn giao_xong_ghi_phai_thu_hang_mot_lan() {
    TestDatabase::run(async |test| {
        let (app, store, account, method) = cod_shop(test, &[]).await;
        let (id, _) = booked(&app, store, &method, "a").await;
        deliver(&app, &id).await;
        assert_eq!(carrier_balance(&app, store, &account).await, 520_000);
        // Job chạy lại: không ghi lần hai.
        let service = service_with_ledger(&app);
        service.post_cod(&app.default_tenant(), Uuid::parse_str(&id).unwrap()).await.unwrap();
        assert_eq!(carrier_balance(&app, store, &account).await, 520_000);
        let (_, detail) = call(&app, "GET", &format!("/shipments/{id}"), "boss", None).await;
        assert_eq!(detail["data"]["cod_status"], "collected");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn bang_ke_khop_ghi_so_cai() {
    TestDatabase::run(async |test| {
        let (app, store, account, method) = cod_shop(test, &[]).await;
        let (id, tracking) = booked(&app, store, &method, "a").await;
        deliver(&app, &id).await;
        let csv = format!("tracking_number,cod_amount,fee\n{tracking},520000,20000\n");
        let (status, statement) = import(&app, &account, "BK-1", &csv).await;
        assert_eq!(status, StatusCode::CREATED, "{statement}");
        assert_eq!(statement["data"]["status"], "reconciled");
        assert_eq!(statement["data"]["net_paid"]["amount"], "500000");
        drain(&app).await;
        assert_eq!(
            carrier_balance(&app, store, &account).await,
            0,
            "the carrier owes nothing more"
        );
        let key = AccountKey::new(store, "shipping.settlement_in_transit", Currency::VND)
            .with_counterparty(Counterparty::new("shipping.carrier_account", account.clone()));
        assert_eq!(balance(&app, key).await, 500_000);
        let fee = AccountKey::new(store, "shipping.carrier_fee", Currency::VND);
        assert_eq!(balance(&app, fee).await, 20_000);
        let (_, detail) = call(&app, "GET", &format!("/shipments/{id}"), "boss", None).await;
        assert_eq!(detail["data"]["cod_status"], "settled");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn bang_ke_lech_tien_sinh_chenh_lech() {
    TestDatabase::run(async |test| {
        let (app, store, account, method) = cod_shop(test, &[]).await;
        let (id, tracking) = booked(&app, store, &method, "a").await;
        deliver(&app, &id).await;
        let csv =
            format!("tracking_number,cod_amount,fee\n{tracking},500000,20000\nNOPE123,10000,0\n");
        let (status, statement) = import(&app, &account, "BK-2", &csv).await;
        assert_eq!(status, StatusCode::CREATED, "{statement}");
        assert_eq!(statement["data"]["status"], "has_mismatch");
        let (_, list) = call(&app, "GET", "/cod/discrepancies?status=open", "boss", None).await;
        let kinds: Vec<&str> = list["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["kind"].as_str().unwrap())
            .collect();
        assert!(
            kinds.contains(&"amount_mismatch") && kinds.contains(&"unknown_tracking"),
            "{list}"
        );
        let (_, detail) = call(&app, "GET", &format!("/shipments/{id}"), "boss", None).await;
        assert_eq!(detail["data"]["cod_status"], "mismatch");
        // Giải quyết: mã lạ bị từ chối; `written_off` ghi sổ phần lệch; giải quyết lần hai → 409.
        let mismatch = list["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["kind"] == "amount_mismatch")
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let path = format!("/cod/discrepancies/{mismatch}/resolve");
        let (status, _) = call(
            &app,
            "POST",
            &path,
            "boss",
            Some(json!({"resolution_code": "nope", "note": "x"})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let resolve = json!({"resolution_code": "written_off", "note": "Hang khong tra du"});
        // `shipping.cod.read` chỉ xem: thấy chênh lệch và bảng kê, không giải quyết được.
        let (status, seen) =
            call(&app, "GET", "/cod/discrepancies?status=open", "auditor", None).await;
        assert_eq!(status, StatusCode::OK, "{seen}");
        assert_eq!(seen["data"], list["data"]);
        let (status, _) = call(&app, "GET", "/cod/statements", "auditor", None).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = call(&app, "POST", &path, "auditor", Some(resolve.clone())).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, resolved) = call(&app, "POST", &path, "boss", Some(resolve.clone())).await;
        assert_eq!(status, StatusCode::OK, "{resolved}");
        assert_eq!(resolved["data"]["resolved_by"], json!(Uuid::from_u128(1)));
        let (status, _) = call(&app, "POST", &path, "boss", Some(resolve)).await;
        assert_eq!(status, StatusCode::CONFLICT);
        drain(&app).await;
        assert_eq!(carrier_balance(&app, store, &account).await, 500_000, "20 000 written off");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ma_van_don_la_trong_bang_ke() {
    TestDatabase::run(async |test| {
        let (app, _store, account, _method) = cod_shop(test, &[]).await;
        let (status, statement) =
            import(&app, &account, "BK-3", "tracking_number,cod_amount,fee\nX1,1000,0\n").await;
        assert_eq!(status, StatusCode::CREATED);
        let (_, detail) = call(
            &app,
            "GET",
            &format!("/cod/statements/{}", statement["data"]["id"].as_str().unwrap()),
            "boss",
            None,
        )
        .await;
        assert_eq!(detail["data"]["lines"][0]["match"], "unknown_tracking");
        // CSV thiếu cột.
        let (status, problem) = import(&app, &account, "BK-4", "code,amount\nX,1\n").await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nhap_lai_bang_ke_khong_nhan_doi() {
    TestDatabase::run(async |test| {
        let (app, store, account, method) = cod_shop(test, &[]).await;
        let (id, tracking) = booked(&app, store, &method, "a").await;
        deliver(&app, &id).await;
        let csv = format!("tracking_number,cod_amount,fee\n{tracking},520000,20000\n");
        let (first, _) = import(&app, &account, "BK-5", &csv).await;
        assert_eq!(first, StatusCode::CREATED);
        let (again, _) = import(&app, &account, "BK-5", &csv).await;
        assert_eq!(again, StatusCode::OK);
        let (changed, problem) =
            import(&app, &account, "BK-5", &csv.replace("20000", "10000")).await;
        assert_eq!(changed, StatusCode::CONFLICT);
        assert_eq!(problem["code"], "shipping.statement_duplicate");
        drain(&app).await;
        assert_eq!(carrier_balance(&app, store, &account).await, 0);
        let (_, list) = call(&app, "GET", "/cod/statements", "boss", None).await;
        assert_eq!(list["data"].as_array().unwrap().len(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cod_qua_han_chua_doi_soat() {
    TestDatabase::run(async |test| {
        let (app, store, _account, method) = cod_shop(test, &[]).await;
        let (id, _) = booked(&app, store, &method, "a").await;
        deliver(&app, &id).await;
        let tenant = app.default_tenant();
        let service = service_with_ledger(&app);
        assert_eq!(service.cod_aging(&tenant).await.unwrap(), 0, "not overdue yet");
        crate::entity::shipment::Entity::update_many()
            .col_expr(
                crate::entity::shipment::Column::DeliveredAt,
                Expr::value(Some(Timestamp::now() - SignedDuration::from_hours(24 * 20))),
            )
            .filter(crate::entity::shipment::Column::Id.eq(Uuid::parse_str(&id).unwrap()))
            .exec(tenant.database().write())
            .await
            .unwrap();
        assert_eq!(service.cod_aging(&tenant).await.unwrap(), 1);
        assert_eq!(service.cod_aging(&tenant).await.unwrap(), 0, "opened once");
        let (_, list) = call(&app, "GET", "/cod/discrepancies", "boss", None).await;
        assert_eq!(list["data"][0]["kind"], "overdue");
        assert_eq!(list["data"][0]["expected"]["amount"], "520000");
    })
    .await
    .unwrap();
}

/// Service gọi thẳng, có `dyn Ledger` của app.
fn service_with_ledger(app: &TestApp) -> Arc<crate::service::ShippingService> {
    let base = service_of(app);
    let mut parts = crate::service::Parts {
        outbox: base.outbox.clone(),
        jobs: base.jobs.clone(),
        entitlements: base.entitlements.clone(),
        databases: base.databases.clone(),
        secret: base.secret.clone(),
        providers: base.providers.clone(),
        carriers: base.carriers.clone(),
        ledger: None,
        reasons: base.reasons.clone(),
        catalog: None,
        orgs: None,
        units: base.units.clone(),
    };
    parts.ledger = app.api::<dyn Ledger>();
    Arc::new(crate::service::ShippingService::new(parts, Default::default()))
}

#[tokio::test]
async fn hai_bang_ke_cung_van_don_chi_ghi_so_mot_lan() {
    TestDatabase::run(async |test| {
        let (app, store, account, method) = cod_shop(test, &[]).await;
        let (id, tracking) = booked(&app, store, &method, "a").await;
        deliver(&app, &id).await;
        let csv = format!("tracking_number,cod_amount,fee\n{tracking},520000,20000\n");
        // Hai bảng kê khác mã cùng có vận đơn này, nhập cùng lúc: Postgres từng cho cả hai cùng
        // thấy vận đơn chưa chốt và ghi có tiền COD hai lần.
        let (first, second) = tokio::join!(
            import(&app, &account, "BK-A", &csv),
            import(&app, &account, "BK-B", &csv)
        );
        assert_eq!(first.0, StatusCode::CREATED, "{}", first.1);
        assert_eq!(second.0, StatusCode::CREATED, "{}", second.1);
        // Một bảng kê lặp dòng của vận đơn khác: cũng chỉ ghi một lần.
        let (other, tracking) = booked(&app, store, &method, "b").await;
        deliver(&app, &other).await;
        let csv = format!(
            "tracking_number,cod_amount,fee\n{tracking},520000,20000\n{tracking},520000,20000\n"
        );
        let (status, statement) = import(&app, &account, "BK-C", &csv).await;
        assert_eq!(status, StatusCode::CREATED, "{statement}");
        drain(&app).await;
        let key = AccountKey::new(store, "shipping.settlement_in_transit", Currency::VND)
            .with_counterparty(Counterparty::new("shipping.carrier_account", account.clone()));
        assert_eq!(balance(&app, key).await, 1_000_000, "each shipment settled once");
        assert_eq!(carrier_balance(&app, store, &account).await, 0);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hai_nguoi_xu_ly_chenh_lech_cung_luc_chi_mot_ben_duoc() {
    TestDatabase::run(async |test| {
        let (app, store, account, method) = cod_shop(test, &[]).await;
        let (id, tracking) = booked(&app, store, &method, "a").await;
        deliver(&app, &id).await;
        let csv = format!("tracking_number,cod_amount,fee\n{tracking},500000,20000\n");
        let (status, statement) = import(&app, &account, "BK-R", &csv).await;
        assert_eq!(status, StatusCode::CREATED, "{statement}");
        let (_, list) = call(&app, "GET", "/cod/discrepancies?status=open", "boss", None).await;
        let mismatch = list["data"][0]["id"].as_str().unwrap().to_owned();
        let path = format!("/cod/discrepancies/{mismatch}/resolve");
        let resolve = json!({"resolution_code": "written_off", "note": "Hang khong tra du"});
        // Hai người bấm giải quyết cùng lúc: Postgres từng cho cả hai cùng thấy `open`, cùng ghi và
        // cùng phát event.
        let (first, second) = tokio::join!(
            call(&app, "POST", &path, "boss", Some(resolve.clone())),
            call(&app, "POST", &path, "boss", Some(resolve.clone())),
        );
        let mut statuses = [first.0, second.0];
        statuses.sort();
        assert_eq!(statuses, [StatusCode::OK, StatusCode::CONFLICT], "{first:?} {second:?}");
        drain(&app).await;
        assert_eq!(
            carrier_balance(&app, store, &account).await,
            500_000,
            "20 000 written off once"
        );
    })
    .await
    .unwrap();
}
