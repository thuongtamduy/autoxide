//! Test thanh toán của `ledger` (cả SQLite và Postgres): webhook sai chữ ký không lưu, webhook gửi
//! lại đồng thời chỉ xử lý một lần, đối tác không trả lời rồi gửi lại cùng `reference`, hoàn tiền
//! đồng thời không vượt số đã thu, đối soát báo thiếu và lệch, xử lý tay một mục chờ.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::event::Actor;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::payment::{PaymentGateway, StatementKind, StatementLine};
use autoxide_kernel::tenancy::{DEFAULT_TENANT, root_of};
use autoxide_testkit::{FakePaymentGateway, TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::{AccountKey, Counterparty, Ledger, NewPayment, NewRefund};

const ALL: [&str; 4] =
    ["ledger.read", "ledger.payments_create", "ledger.refunds_create", "ledger.reconcile"];

async fn app(test: &TestDatabase) -> (TestApp, Arc<FakePaymentGateway>) {
    let gateway = Arc::new(FakePaymentGateway::new());
    let app = TestApp::builder(test)
        .user("cashier", 1, &ALL)
        .contribute::<dyn PaymentGateway>(gateway.clone())
        .module(crate::module())
        .build()
        .await
        .unwrap();
    (app, gateway)
}

fn ledger(app: &TestApp) -> Arc<dyn Ledger> {
    app.api::<dyn Ledger>().unwrap()
}

fn vnd(minor: i64) -> Money {
    Money::new(minor, Currency::VND)
}

async fn call(app: &TestApp, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method, &format!("/api/v1/ledger{uri}")).token("cashier");
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let body = if answer.body().is_empty() { Value::Null } else { answer.json() };
    (answer.status(), body)
}

async fn webhook(app: &TestApp, signed: (Vec<(String, String)>, Vec<u8>)) -> StatusCode {
    let (headers, body) = signed;
    let mut request = app.request(Method::POST, "/api/v1/ledger/webhooks/fake").bytes(body);
    request = request.header("content-type", "application/json");
    for (name, value) in &headers {
        request = request.header(name, value);
    }
    request.send().await.unwrap().status()
}

/// Chạy hết job đang chờ (xử lý thông báo, niêm phong…).
async fn drain(app: &TestApp) {
    let worker = app.modules().worker(4).unwrap().blocked_delay(std::time::Duration::ZERO);
    for _ in 0..5 {
        if worker.run_once().await.unwrap() == 0 {
            break;
        }
    }
}

async fn pay(app: &TestApp, amount: i64, reference: &str) -> crate::api::Payment {
    let tenant = app.default_tenant();
    let payment =
        NewPayment::new(root_of(DEFAULT_TENANT), "card", vnd(amount), "Order", "demo", reference);
    ledger(app).create_payment(&tenant, Actor::system(), payment).await.unwrap().into_inner()
}

async fn clearing(app: &TestApp) -> i64 {
    let key = AccountKey::new(root_of(DEFAULT_TENANT), "clearing.gateway", Currency::VND)
        .with_counterparty(Counterparty::new("partner", "fake"));
    ledger(app).balance(&app.default_tenant(), key, None).await.unwrap().minor()
}

async fn status(app: &TestApp, id: Uuid) -> String {
    ledger(app).payment(&app.default_tenant(), id).await.unwrap().unwrap().status
}

async fn exceptions(app: &TestApp) -> Vec<Value> {
    let (_, listed) = call(app, "GET", "/exceptions?status=open", None).await;
    listed["data"].as_array().unwrap().clone()
}

#[tokio::test]
async fn webhook_sai_chu_ky_tra_401_va_khong_luu() {
    TestDatabase::run(async |test| {
        let (app, gateway) = app(test).await;
        let payment = pay(&app, 100_000, "P-1").await;
        let (headers, mut body) = gateway.paid(payment.id, vnd(100_000), "evt-1");
        body.push(b' ');
        assert_eq!(webhook(&app, (headers, body)).await, StatusCode::UNAUTHORIZED);
        // Chữ ký đúng nhưng quá cửa sổ thời gian: phát lại bị chặn.
        let stale = FakePaymentGateway::signed(
            &json!({"event_id": "evt-2", "kind": "payment_succeeded", "provider_ref": payment.provider_ref,
                    "amount": 100_000, "currency": "VND"}),
            Timestamp::now().as_unix_seconds() - 3600,
        );
        assert_eq!(webhook(&app, stale).await, StatusCode::UNAUTHORIZED);
        drain(&app).await;
        assert_eq!(status(&app, payment.id).await, "pending");
        assert_eq!(clearing(&app).await, 0);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn webhook_gui_lai_dong_thoi_chi_xu_ly_mot_lan() {
    TestDatabase::run(async |test| {
        let (app, gateway) = app(test).await;
        let payment = pay(&app, 250_000, "P-1").await;
        assert!(payment.checkout_url.is_some());
        let signed = gateway.paid(payment.id, vnd(250_000), "evt-1");
        let statuses = futures_like_join(&app, signed, 8).await;
        assert!(statuses.iter().all(|status| *status == StatusCode::OK), "{statuses:?}");
        // Một thông báo khác (mã khác) cho cùng thanh toán cũng không ghi thêm.
        assert_eq!(
            webhook(&app, gateway.paid(payment.id, vnd(250_000), "evt-2")).await,
            StatusCode::OK
        );
        drain(&app).await;
        assert_eq!(status(&app, payment.id).await, "succeeded");
        assert_eq!(clearing(&app).await, 250_000);
        let (_, entries) = call(&app, "GET", "/entries?source_module=ledger", None).await;
        assert_eq!(entries["data"].as_array().unwrap().len(), 1, "{entries}");
        // Thông báo "thất bại" đến muộn không làm lùi trạng thái.
        let failed = gateway.webhook(&json!({"event_id": "evt-3", "kind": "payment_failed",
            "provider_ref": payment.provider_ref, "amount": 250_000, "currency": "VND"}));
        assert_eq!(webhook(&app, failed).await, StatusCode::OK);
        drain(&app).await;
        assert_eq!(status(&app, payment.id).await, "succeeded");
    })
    .await
    .unwrap();
}

/// Gửi cùng webhook `times` lần cùng lúc (các future chạy xen nhau, đủ để đụng nhau ở database).
async fn futures_like_join(
    app: &TestApp,
    signed: (Vec<(String, String)>, Vec<u8>),
    times: usize,
) -> Vec<StatusCode> {
    futures_util::future::join_all((0..times).map(|_| webhook(app, signed.clone()))).await
}

#[tokio::test]
async fn doi_tac_khong_tra_loi_roi_gui_lai_cung_reference() {
    TestDatabase::run(async |test| {
        let (app, gateway) = app(test).await;
        gateway.fail_next(1);
        let body = json!({"method": "card", "amount": {"amount": "50000", "currency": "VND"},
                          "description": "Order 7", "reference": "P-7", "source_id": "order-7"});
        let (status, answer) = call(&app, "POST", "/payments", Some(body.clone())).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{answer}");
        assert_eq!(answer["code"], "payment.provider_unavailable");
        let (status, again) = call(&app, "POST", "/payments", Some(body.clone())).await;
        assert_eq!(status, StatusCode::OK, "{again}");
        assert!(again["data"]["checkout_url"].as_str().is_some(), "{again}");
        let (status, third) = call(&app, "POST", "/payments", Some(body)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(third["data"]["id"], again["data"]["id"]);
        assert_eq!(gateway.payments.lock().unwrap().len(), 1);
        let (_, listed) = call(&app, "GET", "/payments?source_id=order-7", None).await;
        assert_eq!(listed["data"].as_array().unwrap().len(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hoan_tien_dong_thoi_khong_vuot_so_da_thu() {
    TestDatabase::run(async |test| {
        let (app, gateway) = app(test).await;
        let payment = pay(&app, 100_000, "P-1").await;
        assert_eq!(webhook(&app, gateway.paid(payment.id, vnd(100_000), "evt-1")).await, StatusCode::OK);
        drain(&app).await;
        let tenant = app.default_tenant();
        let tasks: Vec<_> = (0..12)
            .map(|index| {
                let (ledger, tenant) = (ledger(&app), tenant.clone());
                tokio::spawn(async move {
                    let refund = NewRefund::new(payment.id, vnd(30_000), "Returned", format!("R-{index}"));
                    ledger.refund(&tenant, Actor::system(), refund).await
                })
            })
            .collect();
        let mut ok = 0;
        for task in tasks {
            match task.await.unwrap() {
                Ok(_) => ok += 1,
                Err(error) => assert_eq!(error.code(), "ledger.refund_exceeds_payment"),
            }
        }
        assert_eq!(ok, 3);
        let found = ledger(&app).payment(&tenant, payment.id).await.unwrap().unwrap();
        assert_eq!(found.refunded.minor(), 90_000);
        assert_eq!(clearing(&app).await, 10_000);
        // Qua REST: vượt số còn lại → 422.
        let (status, answer) = call(
            &app,
            "POST",
            &format!("/payments/{}/refunds", payment.id),
            Some(json!({"amount": {"amount": "20000", "currency": "VND"}, "reason": "More", "reference": "R-X"})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
        assert_eq!(answer["code"], "ledger.refund_exceeds_payment");
        let (_, detail) = call(&app, "GET", &format!("/payments/{}", payment.id), None).await;
        assert_eq!(detail["data"]["refunds"].as_array().unwrap().len(), 3);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn doi_soat_bao_thieu_va_lech_roi_xu_ly_tay() {
    TestDatabase::run(async |test| {
        let (app, gateway) = app(test).await;
        let today = Timestamp::now().date_utc();
        // A: đã thu qua webhook, có trong sao kê (kèm phí). B: webhook lạc, sao kê có → ghi như đã thu.
        // C: đã thu nhưng sao kê không có. D: sao kê báo số khác. E: sao kê có giao dịch lạ.
        let a = pay(&app, 100_000, "A").await;
        let b = pay(&app, 200_000, "B").await;
        let c = pay(&app, 300_000, "C").await;
        let d = pay(&app, 400_000, "D").await;
        for (found, event) in [(&a, "evt-a"), (&c, "evt-c")] {
            let signed = gateway.paid(found.id, found.amount, event);
            assert_eq!(webhook(&app, signed).await, StatusCode::OK);
        }
        drain(&app).await;
        let line = |id: &str, found: &crate::api::Payment, amount: i64| {
            StatementLine::new(
                id,
                found.provider_ref.clone().unwrap(),
                StatementKind::Payment,
                vnd(amount),
                today,
            )
        };
        gateway.set_statement(vec![
            line("L-a", &a, 100_000).fee(vnd(1_100)),
            line("L-b", &b, 200_000),
            line("L-d", &d, 399_000),
            StatementLine::new("L-e", "fake_unknown", StatementKind::Payment, vnd(5_000), today),
        ]);
        let (status, run) =
            call(&app, "POST", "/reconciliations", Some(json!({"date": today}))).await;
        assert_eq!(status, StatusCode::CREATED, "{run}");
        assert_eq!(run["data"]["lines"], 4);
        assert_eq!(run["data"]["matched"], 2);
        assert_eq!(status_of(&app, b.id).await, "succeeded");
        assert_eq!(status_of(&app, d.id).await, "needs_review");
        let open = exceptions(&app).await;
        let kinds: Vec<&str> = open.iter().map(|item| item["kind"].as_str().unwrap()).collect();
        for kind in ["missing_in_statement", "amount_mismatch", "unknown_in_statement"] {
            assert!(kinds.contains(&kind), "{kinds:?}");
        }
        // Phí cổng ghi giảm `clearing.gateway`.
        assert_eq!(clearing(&app).await, 100_000 + 200_000 + 300_000 - 1_100);
        // Chạy lại cùng ngày: không thêm dòng, không mở trùng.
        let (status, _) =
            call(&app, "POST", "/reconciliations", Some(json!({"date": today}))).await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(exceptions(&app).await.len(), open.len());
        assert_eq!(clearing(&app).await, 600_000 - 1_100);
        // Xử lý tay. `match` khi đối tác thu khác số phải thu: từ chối, D vẫn chờ xem xét.
        let mismatch = open.iter().find(|item| item["kind"] == "amount_mismatch").unwrap();
        let resolve_uri = format!("/exceptions/{}/resolve", mismatch["id"].as_str().unwrap());
        let (status, refused) = call(
            &app,
            "POST",
            &resolve_uri,
            Some(json!({"action": "match", "payment_id": d.id, "note": "Partner short-paid"})),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{refused}");
        assert_eq!(refused["code"], "ledger.amount_differs");
        assert_eq!(status_of(&app, d.id).await, "needs_review");
        // Thanh toán khác với thanh toán mục chờ nêu: từ chối, kể cả khi xác nhận riêng.
        let (status, refused) = call(
            &app,
            "POST",
            &resolve_uri,
            Some(json!({"action": "accept_received", "payment_id": c.id})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
        assert_eq!(status_of(&app, c.id).await, "succeeded");
        // Giao dịch lạ 5 000 không ghi được cho thanh toán đã thu, cũng không thành số khác.
        let unknown = open.iter().find(|item| item["kind"] == "unknown_in_statement").unwrap();
        let (status, refused) = call(
            &app,
            "POST",
            &format!("/exceptions/{}/resolve", unknown["id"].as_str().unwrap()),
            Some(json!({"action": "match", "payment_id": a.id})),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{refused}");
        assert_eq!(refused["code"], "ledger.payment_state");
        assert_eq!(clearing(&app).await, 600_000 - 1_100);
        // Xác nhận riêng: ghi đúng số thực thu, event mang số thực thu.
        let (status, resolved) = call(
            &app,
            "POST",
            &resolve_uri,
            Some(json!({"action": "accept_received", "payment_id": d.id, "note": "Short-paid"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{resolved}");
        assert_eq!(resolved["data"]["status"], "resolved");
        assert_eq!(resolved["data"]["resolution"], "accept_received");
        assert_eq!(status_of(&app, d.id).await, "succeeded");
        assert_eq!(clearing(&app).await, 600_000 - 1_100 + 399_000);
        assert_eq!(succeeded_amount(&app, d.id).await, "399000");
    })
    .await
    .unwrap();
}

/// Số tiền (`amount.amount`) của event `ledger.payment_succeeded` cho thanh toán `id`.
async fn succeeded_amount(app: &TestApp, id: Uuid) -> String {
    use sea_orm::{ConnectionTrait, Statement};
    let read = app.database().read();
    let sql = "SELECT payload FROM ledger_events WHERE name = 'ledger.payment_succeeded'";
    let rows =
        read.query_all_raw(Statement::from_string(read.get_database_backend(), sql)).await.unwrap();
    let payloads: Vec<Value> = rows
        .iter()
        .map(|row| serde_json::from_str(&row.try_get::<String>("", "payload").unwrap()).unwrap())
        .filter(|payload: &Value| payload["payment_id"] == json!(id))
        .collect();
    assert_eq!(payloads.len(), 1, "{payloads:?}");
    payloads[0]["amount"]["amount"].as_str().unwrap().to_owned()
}

async fn status_of(app: &TestApp, id: Uuid) -> String {
    status(app, id).await
}

#[tokio::test]
async fn so_tien_khac_va_tien_ve_tre_vao_hang_cho() {
    TestDatabase::run(async |test| {
        let (app, gateway) = app(test).await;
        let short = pay(&app, 100_000, "S").await;
        assert_eq!(
            webhook(&app, gateway.paid(short.id, vnd(90_000), "evt-s")).await,
            StatusCode::OK
        );
        drain(&app).await;
        assert_eq!(status(&app, short.id).await, "needs_review");
        assert_eq!(clearing(&app).await, 0);
        let kinds: Vec<String> = exceptions(&app)
            .await
            .iter()
            .map(|item| item["kind"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(kinds, ["amount_mismatch"]);
        // Không cấu hình cổng → lỗi rõ.
        let tenant = app.default_tenant();
        let other = NewPayment::new(root_of(DEFAULT_TENANT), "card", vnd(1), "X", "demo", "N")
            .provider("vnpay");
        let error = ledger(&app).create_payment(&tenant, Actor::system(), other).await.unwrap_err();
        assert_eq!(error.code(), "ledger.gateway_not_configured");
    })
    .await
    .unwrap();
}

/// Rà soát bảo mật 2026-11 (L-6): nhận thiếu rồi người duyệt chấp nhận số đã nhận — trần hoàn là số
/// đã thu, không phải số phải thu (hoàn theo số phải thu là trả ra nhiều hơn tiền đã vào).
#[tokio::test]
async fn nhan_thieu_roi_chap_nhan_thi_hoan_toi_da_so_da_thu() {
    TestDatabase::run(async |test| {
        let (app, gateway) = app(test).await;
        let short = pay(&app, 100_000, "S").await;
        assert_eq!(
            webhook(&app, gateway.paid(short.id, vnd(90_000), "evt-s")).await,
            StatusCode::OK
        );
        drain(&app).await;
        let open = exceptions(&app).await;
        let uri = format!("/exceptions/{}/resolve", open[0]["id"].as_str().unwrap());
        let body = json!({"action": "accept_received", "payment_id": short.id});
        let (code, resolved) = call(&app, "POST", &uri, Some(body)).await;
        assert_eq!(code, StatusCode::OK, "{resolved}");
        assert_eq!(status(&app, short.id).await, "succeeded");
        let tenant = app.default_tenant();
        let too_much = NewRefund::new(short.id, vnd(95_000), "Returned", "R-1");
        let error = ledger(&app).refund(&tenant, Actor::system(), too_much).await.unwrap_err();
        assert_eq!(error.code(), "ledger.refund_exceeds_payment");
        let all = NewRefund::new(short.id, vnd(90_000), "Returned", "R-2");
        ledger(&app).refund(&tenant, Actor::system(), all).await.unwrap();
    })
    .await
    .unwrap();
}

/// UT-6: đối tác báo hỏng rồi lại báo đã thu (sai thứ tự): vẫn ghi thu, và mở `late_payment` ở hàng chờ
/// như tiền về sau hạn — module nguồn có thể đã mở lượt thu khác, người vận hành phải thấy. Hoàn đủ qua
/// đối tác thì mục tự đóng; chưa hoàn (vd đối tác từ chối) thì mục còn mở.
#[tokio::test]
async fn hong_roi_lai_bao_da_thu_thi_vao_hang_cho() {
    TestDatabase::run(async |test| {
        let (app, gateway) = app(test).await;
        let payment = pay(&app, 100_000, "F").await;
        let provider_ref = gateway.provider_ref(payment.id).unwrap();
        let failed = gateway.webhook(&json!({
            "event_id": "evt-f", "kind": "payment_failed", "provider_ref": provider_ref,
            "payment_id": payment.id, "amount": 100_000, "currency": "VND", "reason": "declined",
        }));
        assert_eq!(webhook(&app, failed).await, StatusCode::OK);
        drain(&app).await;
        assert_eq!(status(&app, payment.id).await, "failed");
        assert_eq!(
            webhook(&app, gateway.paid(payment.id, vnd(100_000), "evt-p")).await,
            StatusCode::OK
        );
        drain(&app).await;
        assert_eq!(status(&app, payment.id).await, "succeeded");
        assert_eq!(clearing(&app).await, 100_000);
        let open = exceptions(&app).await;
        assert_eq!(open.len(), 1, "{open:?}");
        assert_eq!(open[0]["kind"], "late_payment");
        assert_eq!(open[0]["subject"], format!("ledger.payment:{}", payment.id));

        // Hoàn một phần: mục vẫn mở. Module nguồn hoàn nốt qua đối tác: mục tự đóng, không người xử lý.
        let tenant = app.default_tenant();
        let part = NewRefund::new(payment.id, vnd(40_000), "superseded_payment", "R-part");
        ledger(&app).refund(&tenant, Actor::system(), part).await.unwrap();
        assert_eq!(exceptions(&app).await.len(), 1);
        let rest = NewRefund::new(payment.id, vnd(60_000), "superseded_payment", "R-rest");
        ledger(&app).refund(&tenant, Actor::system(), rest).await.unwrap();
        assert!(exceptions(&app).await.is_empty());
        let (_, closed) = call(&app, "GET", "/exceptions?status=resolved", None).await;
        let closed = &closed["data"][0];
        assert_eq!(closed["kind"], "late_payment", "{closed}");
        assert_eq!(closed["resolution"], "refunded");
        assert!(closed["resolved_by"].is_null());
    })
    .await
    .unwrap();
}

/// UT-6 (rà #591): `expires_by` rút hạn của lượt thu (không kéo dài); đã qua thì từ chối. Tiền về sau hạn
/// khi lượt thu còn `pending` (job hết hạn chưa chạy) vẫn mở `late_payment`.
#[tokio::test]
async fn expires_by_rut_han_va_tien_ve_sau_han_vao_hang_cho() {
    use crate::entity::payment;
    use autoxide_kernel::tenancy::ScopeExt as _;
    use jiff::SignedDuration;
    use sea_orm::sea_query::Expr;
    use sea_orm::{ColumnTrait as _, EntityTrait as _, QueryFilter as _};

    TestDatabase::run(async |test| {
        let (app, gateway) = app(test).await;
        let tenant = app.default_tenant();
        let soon = Timestamp::now() + SignedDuration::from_mins(5);
        let capped =
            NewPayment::new(root_of(DEFAULT_TENANT), "card", vnd(100_000), "Order", "demo", "C")
                .expires_by(soon);
        let made = ledger(&app).create_payment(&tenant, Actor::system(), capped).await.unwrap();
        let made = made.into_inner();
        assert!(made.expires_at <= soon, "{} > {soon}", made.expires_at);
        let later = Timestamp::now() + SignedDuration::from_hours(24 * 30);
        let long = NewPayment::new(root_of(DEFAULT_TENANT), "card", vnd(1), "Order", "demo", "L")
            .expires_by(later);
        let long = ledger(&app).create_payment(&tenant, Actor::system(), long).await.unwrap();
        assert!(long.into_inner().expires_at < later, "không kéo dài quá hạn thường");
        let past = NewPayment::new(root_of(DEFAULT_TENANT), "card", vnd(1), "Order", "demo", "P")
            .expires_by(Timestamp::now() - SignedDuration::from_mins(1));
        let refused =
            ledger(&app).create_payment(&tenant, Actor::system(), past).await.unwrap_err();
        assert_eq!(refused.code(), "validation_failed");

        // Hạn đã qua mà job hết hạn chưa chạy (còn `pending`): tiền về vẫn là muộn.
        let write = tenant.db().begin_write().await.unwrap();
        payment::Entity::update_many()
            .col_expr(
                payment::Column::ExpiresAt,
                Expr::value(Timestamp::now() - SignedDuration::from_mins(1)),
            )
            .filter(payment::Column::Id.eq(made.id))
            .scoped(&tenant)
            .exec(&write)
            .await
            .unwrap();
        write.commit().await.unwrap();
        assert_eq!(status(&app, made.id).await, "pending");
        assert_eq!(
            webhook(&app, gateway.paid(made.id, vnd(100_000), "evt-c")).await,
            StatusCode::OK
        );
        drain(&app).await;
        assert_eq!(status(&app, made.id).await, "succeeded");
        let open = exceptions(&app).await;
        assert_eq!(open.len(), 1, "{open:?}");
        assert_eq!(open[0]["kind"], "late_payment");
    })
    .await
    .unwrap();
}

/// UT-6 (rà #591, M3): đối tác báo từ chối một lượt hoàn → lượt hoàn `failed`, trả số đã giữ chỗ, và mở mục
/// `refund_failed` ở hàng chờ (một lần dù thông báo tới lại). Lượt hoàn sau qua đối tác trả đủ khoản thì mục
/// tự đóng.
#[tokio::test]
async fn doi_tac_tu_choi_hoan_thi_mo_muc_refund_failed() {
    TestDatabase::run(async |test| {
        let (app, gateway) = app(test).await;
        let tenant = app.default_tenant();
        let payment = pay(&app, 100_000, "RF").await;
        assert_eq!(
            webhook(&app, gateway.paid(payment.id, vnd(100_000), "evt-p")).await,
            StatusCode::OK
        );
        drain(&app).await;
        assert!(exceptions(&app).await.is_empty());

        gateway.refunds_pending(true);
        let first = NewRefund::new(payment.id, vnd(100_000), "cancelled", "R-1");
        let first = ledger(&app).refund(&tenant, Actor::system(), first).await.unwrap();
        let first = first.into_inner();
        assert_eq!(first.status, "pending");
        let refused = json!({
            "event_id": "evt-rf", "kind": "refund_failed", "payment_id": payment.id,
            "provider_ref": gateway.provider_ref(payment.id).unwrap(),
            "refund_provider_ref": format!("fake_refund_{}", first.id.simple()),
            "amount": 100_000, "currency": "VND",
        });
        for _ in 0..2 {
            assert_eq!(webhook(&app, gateway.webhook(&refused)).await, StatusCode::OK);
            drain(&app).await;
        }
        let open = exceptions(&app).await;
        assert_eq!(open.len(), 1, "{open:?}");
        assert_eq!(open[0]["kind"], "refund_failed");
        assert_eq!(open[0]["subject"], format!("ledger.payment:{}", payment.id));
        assert_eq!(open[0]["details"]["refund_id"], json!(first.id));

        // Số đã giữ chỗ được trả: hoàn lại đủ khoản qua đối tác được, và mục tự đóng.
        gateway.refunds_pending(false);
        let again = NewRefund::new(payment.id, vnd(100_000), "cancelled", "R-2");
        let again = ledger(&app).refund(&tenant, Actor::system(), again).await.unwrap();
        assert_eq!(again.into_inner().status, "succeeded");
        assert!(exceptions(&app).await.is_empty());
        let (_, closed) = call(&app, "GET", "/exceptions?status=resolved", None).await;
        assert_eq!(closed["data"][0]["kind"], "refund_failed", "{closed}");
        assert_eq!(closed["data"][0]["resolution"], "refunded");
    })
    .await
    .unwrap();
}

/// UT-6 (rà #591, M3): lượt thu về muộn đã có mục `late_payment` mở; lượt hoàn của nó bị từ chối thì không
/// mở thêm mục thứ hai cho cùng khoản tiền.
#[tokio::test]
async fn hoan_tien_ve_muon_bi_tu_choi_khong_mo_muc_trung() {
    TestDatabase::run(async |test| {
        let (app, gateway) = app(test).await;
        let tenant = app.default_tenant();
        let payment = pay(&app, 100_000, "LF").await;
        let provider_ref = gateway.provider_ref(payment.id).unwrap();
        let failed = gateway.webhook(&json!({
            "event_id": "evt-f", "kind": "payment_failed", "provider_ref": provider_ref,
            "payment_id": payment.id, "amount": 100_000, "currency": "VND", "reason": "declined",
        }));
        assert_eq!(webhook(&app, failed).await, StatusCode::OK);
        drain(&app).await;
        assert_eq!(
            webhook(&app, gateway.paid(payment.id, vnd(100_000), "evt-p")).await,
            StatusCode::OK
        );
        drain(&app).await;
        gateway.refunds_pending(true);
        let refund = NewRefund::new(payment.id, vnd(100_000), "late_payment", "R-late");
        let refund = ledger(&app).refund(&tenant, Actor::system(), refund).await.unwrap();
        let refused = json!({
            "event_id": "evt-rf", "kind": "refund_failed", "payment_id": payment.id,
            "provider_ref": provider_ref,
            "refund_provider_ref": format!("fake_refund_{}", refund.into_inner().id.simple()),
            "amount": 100_000, "currency": "VND",
        });
        assert_eq!(webhook(&app, gateway.webhook(&refused)).await, StatusCode::OK);
        drain(&app).await;
        let open = exceptions(&app).await;
        assert_eq!(open.len(), 1, "{open:?}");
        assert_eq!(open[0]["kind"], "late_payment");
    })
    .await
    .unwrap();
}

/// UT-6 (rà #591, M2): cùng luật "về muộn" ở mọi đường — mục chờ ghi được thành "đã thu" cho thanh toán đối
/// tác đã báo hỏng (trước đây trả 409 `ledger.payment_state`), và việc ghi đó mở `late_payment`.
#[tokio::test]
async fn muc_cho_ghi_thu_duoc_cho_thanh_toan_da_hong() {
    TestDatabase::run(async |test| {
        let (app, gateway) = app(test).await;
        let payment = pay(&app, 100_000, "MF").await;
        let provider_ref = gateway.provider_ref(payment.id).unwrap();
        let failed = gateway.webhook(&json!({
            "event_id": "evt-f", "kind": "payment_failed", "provider_ref": provider_ref,
            "payment_id": payment.id, "amount": 100_000, "currency": "VND", "reason": "declined",
        }));
        assert_eq!(webhook(&app, failed).await, StatusCode::OK);
        drain(&app).await;
        assert_eq!(status(&app, payment.id).await, "failed");
        // Đối tác báo thu sai số: mục `amount_mismatch` (thanh toán giữ `failed`).
        assert_eq!(
            webhook(&app, gateway.paid(payment.id, vnd(90_000), "evt-p")).await,
            StatusCode::OK
        );
        drain(&app).await;
        let open = exceptions(&app).await;
        let item = open.iter().find(|item| item["kind"] == "amount_mismatch").unwrap();
        let (code, body) = call(
            &app,
            "POST",
            &format!("/exceptions/{}/resolve", item["id"].as_str().unwrap()),
            Some(json!({"action": "accept_received", "payment_id": payment.id})),
        )
        .await;
        assert_eq!(code, StatusCode::OK, "{body}");
        assert_eq!(status(&app, payment.id).await, "succeeded");
        let open = exceptions(&app).await;
        assert!(open.iter().any(|item| item["kind"] == "late_payment"), "{open:?}");
    })
    .await
    .unwrap();
}
