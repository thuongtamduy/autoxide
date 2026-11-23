//! Test VietQR tự khớp (cả SQLite và Postgres): mã QR đúng chuẩn, biến động gửi lại chỉ ghi một
//! lần, khớp đúng mã + số tiền, sai số tiền / không có mã / tiền lần hai / tiền đến muộn vào hàng
//! chờ, xử lý tay ghi như đã thu, kiểu kéo theo con trỏ, webhook sai chữ ký.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::job::{EnqueueOptions, Job, JobQueue};
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::payment::BankFeed;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, root_of};
use autoxide_testkit::{FakeBankFeed, TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::api::{AccountKey, Counterparty, Ledger};
use crate::entity::payment;

const ALL: [&str; 4] =
    ["ledger.read", "ledger.settings_manage", "ledger.payments_create", "ledger.reconcile"];
const ACCOUNT: &str = "0011001234567";

async fn setup(test: &TestDatabase) -> (TestApp, Arc<FakeBankFeed>, String) {
    let feed = Arc::new(FakeBankFeed::new());
    let app = TestApp::builder(test)
        .user("cashier", 1, &ALL)
        .contribute::<dyn BankFeed>(feed.clone())
        .module(crate::module())
        .build()
        .await
        .unwrap();
    let (status, account) = call(
        &app,
        "POST",
        "/bank-accounts",
        Some(json!({"bank_bin": "970436", "account_number": ACCOUNT, "account_name": "CUA HANG A",
                    "feed_provider": "fake"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{account}");
    let id = account["data"]["id"].as_str().unwrap().to_owned();
    (app, feed, id)
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

fn vnd(minor: i64) -> Money {
    Money::new(minor, Currency::VND)
}

async fn pay(app: &TestApp, amount: i64, reference: &str) -> Value {
    let (status, created) = call(
        app,
        "POST",
        "/payments/vietqr",
        Some(json!({"amount": {"amount": amount.to_string(), "currency": "VND"},
                    "description": "Order", "source_id": "O-1", "reference": reference})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    created["data"].clone()
}

async fn push(
    app: &TestApp,
    transactions: &[autoxide_kernel::payment::BankTransaction],
) -> (StatusCode, Value) {
    let (headers, body) = FakeBankFeed::webhook(transactions);
    let mut request = app.request(Method::POST, "/api/v1/ledger/bank-feeds/fake").bytes(body);
    request = request.header("content-type", "application/json");
    for (name, value) in &headers {
        request = request.header(name, value);
    }
    let answer = request.send().await.unwrap();
    let body = if answer.body().is_empty() { Value::Null } else { answer.json() };
    (answer.status(), body)
}

async fn drain(app: &TestApp) {
    let worker = app.modules().worker(4).unwrap().blocked_delay(std::time::Duration::ZERO);
    for _ in 0..5 {
        if worker.run_once().await.unwrap() == 0 {
            break;
        }
    }
}

async fn status(app: &TestApp, id: &str) -> String {
    let (_, found) = call(app, "GET", &format!("/payments/{id}"), None).await;
    found["data"]["status"].as_str().unwrap().to_owned()
}

async fn exceptions(app: &TestApp) -> Vec<Value> {
    let (_, listed) = call(app, "GET", "/exceptions?status=open", None).await;
    listed["data"].as_array().unwrap().clone()
}

async fn bank_balance(app: &TestApp, account: &str) -> i64 {
    let key = AccountKey::new(root_of(DEFAULT_TENANT), "cash.bank", Currency::VND)
        .with_counterparty(Counterparty::new("bank_account", account));
    let ledger = app.api::<dyn Ledger>().unwrap();
    ledger.balance(&app.default_tenant(), key, None).await.unwrap().minor()
}

#[derive(Debug, Serialize, Deserialize)]
struct PollNow {}

impl Job for PollNow {
    const KIND: &'static str = crate::bank::POLL_BANK_FEED;
}

#[tokio::test]
async fn ma_vietqr_dung_chuan_va_khop_dung_ma_so_tien() {
    TestDatabase::run(async |test| {
        let (app, _feed, account) = setup(test).await;
        let created = pay(&app, 150_000, "P-1").await;
        let id = created["id"].as_str().unwrap().to_owned();
        let code = created["transfer_code"].as_str().unwrap().to_owned();
        assert!(code.starts_with("AX") && code.len() == 12, "{created}");
        assert!(created["expires_at"].is_string());
        // Chuỗi QR: tài khoản, số tiền, mã trong nội dung, CRC đúng.
        let qr = created["qr_payload"].as_str().unwrap();
        assert!(qr.contains(ACCOUNT) && qr.contains("5406150000") && qr.contains(&code), "{qr}");
        let (body, crc) = qr.split_at(qr.len() - 4);
        assert_eq!(format!("{:04X}", crate::vietqr::crc16(body.as_bytes())), crc);
        // Ngân hàng ghi nội dung chữ thường, chèn khoảng trắng.
        let memo = format!("MBVCB.1234 {} thanh toan", code.to_lowercase().replacen('x', "x ", 1));
        let txn = FakeBankFeed::credit("T-1", ACCOUNT, vnd(150_000), &memo);
        let (answer, stored) = push(&app, std::slice::from_ref(&txn)).await;
        assert_eq!(answer, StatusCode::OK, "{stored}");
        assert_eq!(stored["data"]["stored"], 1);
        // Gửi lại cùng biến động: không ghi lần hai.
        let (_, again) = push(&app, std::slice::from_ref(&txn)).await;
        assert_eq!(again["data"]["stored"], 0);
        drain(&app).await;
        assert_eq!(status(&app, &id).await, "succeeded");
        assert_eq!(bank_balance(&app, &account).await, 150_000);
        assert!(exceptions(&app).await.is_empty());
        // Tiền lần hai cho thanh toán đã xong: hàng chờ, không ghi thêm.
        let second = FakeBankFeed::credit("T-2", ACCOUNT, vnd(150_000), &code);
        push(&app, &[second]).await;
        drain(&app).await;
        let open = exceptions(&app).await;
        assert_eq!(open.len(), 1);
        assert_eq!(open[0]["kind"], "duplicate_payment");
        assert_eq!(bank_balance(&app, &account).await, 150_000);
        // Gửi lại cùng reference: cùng thanh toán, cùng mã.
        let (status_code, replay) = call(
            &app,
            "POST",
            "/payments/vietqr",
            Some(json!({"amount": {"amount": "150000", "currency": "VND"},
                        "description": "Order", "source_id": "O-1", "reference": "P-1"})),
        )
        .await;
        assert_eq!(status_code, StatusCode::OK);
        assert_eq!(replay["data"]["transfer_code"], code.as_str());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn sai_so_tien_va_khong_ma_vao_hang_cho_roi_xu_ly_tay() {
    TestDatabase::run(async |test| {
        let (app, _feed, account) = setup(test).await;
        let created = pay(&app, 200_000, "P-1").await;
        let (id, code) = (
            created["id"].as_str().unwrap().to_owned(),
            created["transfer_code"].as_str().unwrap().to_owned(),
        );
        // Sai số tiền.
        push(&app, &[FakeBankFeed::credit("T-1", ACCOUNT, vnd(190_000), &code)]).await;
        // Không có mã.
        push(&app, &[FakeBankFeed::credit("T-2", ACCOUNT, vnd(50_000), "chuyen tien hang")]).await;
        // Tiền ra: bỏ qua.
        let debit = autoxide_kernel::payment::BankTransaction::new(
            "T-3",
            ACCOUNT,
            autoxide_kernel::payment::BankDirection::Debit,
            vnd(10_000),
            "phi",
            Timestamp::now(),
        );
        push(&app, &[debit]).await;
        drain(&app).await;
        assert_eq!(status(&app, &id).await, "needs_review");
        let open = exceptions(&app).await;
        let kinds: Vec<&str> = open.iter().map(|row| row["kind"].as_str().unwrap()).collect();
        assert!(
            kinds.contains(&"amount_mismatch") && kinds.contains(&"unmatched_credit"),
            "{kinds:?}"
        );
        assert_eq!(open.len(), 2);
        assert_eq!(bank_balance(&app, &account).await, 0);
        // Người xem xét, ghi khoản không mã cho thanh toán: số khác số phải thu nên `match` bị từ
        // chối, phải xác nhận riêng (`accept_received`) mới ghi số tiền thực nhận.
        let unmatched = open.iter().find(|row| row["kind"] == "unmatched_credit").unwrap();
        let uri = format!("/exceptions/{}/resolve", unmatched["id"].as_str().unwrap());
        let (code_status, refused) =
            call(&app, "POST", &uri, Some(json!({"action": "match", "payment_id": id}))).await;
        assert_eq!(code_status, StatusCode::CONFLICT, "{refused}");
        assert_eq!(refused["code"], "ledger.amount_differs");
        assert_eq!(status(&app, &id).await, "needs_review");
        let (code_status, resolved) =
            call(&app, "POST", &uri, Some(json!({"action": "accept_received", "payment_id": id})))
                .await;
        assert_eq!(code_status, StatusCode::OK, "{resolved}");
        assert_eq!(status(&app, &id).await, "succeeded");
        assert_eq!(bank_balance(&app, &account).await, 50_000);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tien_den_muon_va_keo_bien_dong_theo_con_tro() {
    TestDatabase::run(async |test| {
        let (app, feed, account) = setup(test).await;
        let created = pay(&app, 80_000, "P-1").await;
        let (id, code) = (
            created["id"].as_str().unwrap().to_owned(),
            created["transfer_code"].as_str().unwrap().to_owned(),
        );
        // Hết hạn (cùng hạn thanh toán với mọi thanh toán).
        // tenant-scope: việc nền / kiểm tra quét mọi khách của database, chủ ý không lọc khách (rà soát bảo mật 2026-11)
        payment::Entity::update_many()
            .col_expr(payment::Column::Status, "expired".into())
            .filter(payment::Column::Id.eq(id.parse::<uuid::Uuid>().unwrap()))
            .exec(app.database().write())
            .await
            .unwrap();
        // Kiểu kéo: nguồn trả biến động, lịch đọc theo con trỏ.
        feed.push(FakeBankFeed::credit("T-1", ACCOUNT, vnd(80_000), &format!("ND {code}")));
        JobQueue::new("ledger")
            .enqueue_database(app.database().write(), &PollNow {}, EnqueueOptions::default())
            .await
            .unwrap();
        drain(&app).await;
        assert_eq!(status(&app, &id).await, "succeeded");
        assert_eq!(bank_balance(&app, &account).await, 80_000);
        let open = exceptions(&app).await;
        assert_eq!(open.len(), 1);
        assert_eq!(open[0]["kind"], "late_payment");
        // Kéo lại: con trỏ đã qua, không ghi lần hai.
        JobQueue::new("ledger")
            .enqueue_database(app.database().write(), &PollNow {}, EnqueueOptions::default())
            .await
            .unwrap();
        drain(&app).await;
        assert_eq!(bank_balance(&app, &account).await, 80_000);
        assert!(feed.fetches.load(std::sync::atomic::Ordering::SeqCst) >= 2);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn webhook_sai_chu_ky_va_tai_khoan_khong_hop_le() {
    TestDatabase::run(async |test| {
        let (app, _feed, _account) = setup(test).await;
        let (headers, mut body) =
            FakeBankFeed::webhook(&[FakeBankFeed::credit("T-1", ACCOUNT, vnd(1), "x")]);
        body.push(b' ');
        let mut request = app.request(Method::POST, "/api/v1/ledger/bank-feeds/fake").bytes(body);
        for (name, value) in &headers {
            request = request.header(name, value);
        }
        assert_eq!(request.send().await.unwrap().status(), StatusCode::UNAUTHORIZED);
        // Ngân hàng không có trong danh sách BIN, tài khoản trùng.
        let (status_code, invalid) = call(
            &app,
            "POST",
            "/bank-accounts",
            Some(json!({"bank_bin": "000000", "account_number": "12 3", "account_name": "X"})),
        )
        .await;
        assert_eq!(status_code, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(invalid["errors"].as_array().unwrap().len(), 2, "{invalid}");
        let (status_code, _) = call(
            &app,
            "POST",
            "/bank-accounts",
            Some(json!({"bank_bin": "970436", "account_number": ACCOUNT, "account_name": "X"})),
        )
        .await;
        assert_eq!(status_code, StatusCode::CONFLICT);
        // Chỉ VND.
        let (status_code, _) = call(
            &app,
            "POST",
            "/payments/vietqr",
            Some(json!({"amount": {"amount": "10", "currency": "USD"}, "description": "x", "reference": "P-9"})),
        )
        .await;
        assert_eq!(status_code, StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn sua_tai_khoan_ngan_hang_tat_thi_khong_nhan_thanh_toan_moi() {
    TestDatabase::run(async |test| {
        let (app, _feed, account) = setup(test).await;
        let uri = format!("/bank-accounts/{account}");

        // Sai cả ba trường: báo một lần.
        let (status, problem) = call(
            &app,
            "PATCH",
            &uri,
            Some(json!({"account_name": "", "feed_provider": "nope", "status": "closed"})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        let fields: Vec<&str> = problem["errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["field"].as_str().unwrap())
            .collect();
        assert_eq!(fields, ["account_name", "feed_provider", "status"]);

        // Đổi tên, bỏ nguồn biến động; số tài khoản giữ nguyên.
        let (status, changed) = call(
            &app,
            "PATCH",
            &uri,
            Some(json!({"account_name": "  CUA HANG B ", "feed_provider": null})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{changed}");
        assert_eq!(changed["data"]["account_name"], "CUA HANG B");
        assert_eq!(changed["data"]["feed_provider"], Value::Null);
        assert_eq!(changed["data"]["account_number"], ACCOUNT);
        assert_eq!(changed["data"]["status"], "active");

        // Không được đổi số tài khoản.
        let (status, _) = call(&app, "PATCH", &uri, Some(json!({"account_number": "123"}))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

        // Tắt: thanh toán VietQR mới không còn tài khoản nhận.
        let (status, changed) =
            call(&app, "PATCH", &uri, Some(json!({"status": "disabled"}))).await;
        assert_eq!(status, StatusCode::OK, "{changed}");
        assert_eq!(changed["data"]["status"], "disabled");
        let (status, problem) = call(
            &app,
            "POST",
            "/payments/vietqr",
            Some(json!({"amount": {"amount": "1000", "currency": "VND"},
                        "description": "Order", "source_id": "O-2", "reference": "R-2"})),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{problem}");
        assert_eq!(problem["code"], "ledger.bank_account_not_found");

        let (status, problem) = call(
            &app,
            "PATCH",
            &format!("/bank-accounts/{}", uuid::Uuid::now_v7()),
            Some(json!({"status": "active"})),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(problem["code"], "ledger.bank_account_not_found");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn loc_thanh_toan_theo_so_tien_tien_te_va_nut() {
    TestDatabase::run(async |test| {
        let (app, _feed, _account) = setup(test).await;
        let small = pay(&app, 150_000, "P-1").await;
        pay(&app, 990_000, "P-2").await;

        let ids = |listed: &Value| -> Vec<String> {
            listed["data"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["id"].as_str().unwrap().to_owned())
                .collect()
        };
        let (status, listed) =
            call(&app, "GET", "/payments?amount=150000&currency=VND&status=pending", None).await;
        assert_eq!(status, StatusCode::OK, "{listed}");
        assert_eq!(ids(&listed), [small["id"].as_str().unwrap()]);

        let root = root_of(DEFAULT_TENANT).as_uuid();
        let (_, listed) = call(&app, "GET", &format!("/payments?org_id={root}"), None).await;
        assert_eq!(ids(&listed).len(), 2);
        let (_, listed) =
            call(&app, "GET", &format!("/payments?org_id={}", uuid::Uuid::now_v7()), None).await;
        assert!(ids(&listed).is_empty());
        let (_, listed) = call(&app, "GET", "/payments?currency=USD", None).await;
        assert!(ids(&listed).is_empty());

        // Mọi tham số sai báo một lần.
        let (status, problem) =
            call(&app, "GET", "/payments?amount=1.5&currency=XYZ&limit=0", None).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        let fields: Vec<&str> = problem["errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["field"].as_str().unwrap())
            .collect();
        assert_eq!(fields, ["amount", "currency", "limit"]);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn xem_truoc_vietqr_khong_tao_thanh_toan() {
    TestDatabase::run(async |test| {
        let (app, _feed, account) = setup(test).await;
        let uri = format!("/bank-accounts/{account}/vietqr-preview");
        let (status, preview) = call(
            &app,
            "POST",
            &uri,
            Some(json!({"amount": {"amount": "10000", "currency": "VND"}, "memo": "THU 1"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{preview}");
        let payload = preview["data"]["qr_payload"].as_str().unwrap();
        assert!(payload.contains(ACCOUNT) && payload.contains("970436"), "{payload}");
        assert!(payload.contains("THU 1"));
        assert_eq!(preview["data"]["bank_account"]["id"], account.as_str());
        // Không nêu nội dung: mã chuyển khoản mẫu.
        let (_, sample) = call(
            &app,
            "POST",
            &uri,
            Some(json!({"amount": {"amount": "5000", "currency": "VND"}})),
        )
        .await;
        assert!(sample["data"]["memo"].as_str().unwrap().starts_with("AX"), "{sample}");
        // Không có thanh toán nào được tạo.
        let (_, listed) = call(&app, "GET", "/payments", None).await;
        assert!(listed["data"].as_array().unwrap().is_empty());

        let (status, problem) = call(
            &app,
            "POST",
            &uri,
            Some(json!({"amount": {"amount": "5", "currency": "USD"}, "memo": "có dấu"})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        let fields: Vec<&str> = problem["errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["field"].as_str().unwrap())
            .collect();
        assert_eq!(fields, ["amount", "memo"]);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hai_lan_chuyen_cung_ma_khop_dong_thoi_khong_mat_khoan_thua() {
    TestDatabase::run(async |test| {
        let (app, _feed, account) = setup(test).await;
        let created = pay(&app, 150_000, "P-1").await;
        let id = created["id"].as_str().unwrap().to_owned();
        let code = created["transfer_code"].as_str().unwrap().to_owned();
        // Hai lần chuyển cho cùng mã tới cùng lúc, hai worker khớp song song.
        let first = FakeBankFeed::credit("T-1", ACCOUNT, vnd(150_000), &code);
        let second = FakeBankFeed::credit("T-2", ACCOUNT, vnd(150_000), &code);
        let (answer, stored) = push(&app, &[first, second]).await;
        assert_eq!(answer, StatusCode::OK, "{stored}");
        tokio::join!(drain(&app), drain(&app));
        drain(&app).await;
        // Một lần ghi đã thu; lần kia vào hàng chờ (không mất, không cộng hai lần).
        assert_eq!(status(&app, &id).await, "succeeded");
        assert_eq!(bank_balance(&app, &account).await, 150_000);
        let open = exceptions(&app).await;
        let kinds: Vec<&str> = open.iter().map(|item| item["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["duplicate_payment"], "{open:?}");
    })
    .await
    .unwrap();
}

/// Rà soát bảo mật 2026-11: tiền vào một tài khoản khác (của người bán khác) mang mã của thanh toán
/// không làm thanh toán đó thành đã nhận — tiền không nằm ở tài khoản nó chờ; khoản đó vào hàng chờ.
#[tokio::test]
async fn ma_dung_nhung_sai_tai_khoan_nhan_vao_hang_cho() {
    TestDatabase::run(async |test| {
        let (app, _feed, account) = setup(test).await;
        let created = pay(&app, 150_000, "P-9").await;
        let id = created["id"].as_str().unwrap().to_owned();
        let code = created["transfer_code"].as_str().unwrap().to_owned();
        const OTHER: &str = "0022002345678";
        let (status_code, other) = call(
            &app,
            "POST",
            "/bank-accounts",
            Some(json!({"bank_bin": "970436", "account_number": OTHER,
                        "account_name": "CUA HANG B", "feed_provider": "fake"})),
        )
        .await;
        assert_eq!(status_code, StatusCode::CREATED, "{other}");
        let other_id = other["data"]["id"].as_str().unwrap().to_owned();
        let txn = FakeBankFeed::credit("T-9", OTHER, vnd(150_000), &code);
        let (answer, stored) = push(&app, &[txn]).await;
        assert_eq!(answer, StatusCode::OK, "{stored}");
        drain(&app).await;
        assert_eq!(status(&app, &id).await, "pending");
        assert_eq!(bank_balance(&app, &account).await, 0);
        let open = exceptions(&app).await;
        assert_eq!(open.len(), 1, "{open:?}");
        assert_eq!(open[0]["kind"], "unmatched_credit");
        assert_ne!(other_id, account);
    })
    .await
    .unwrap();
}

/// Rà soát bảo mật 2026-11: hai lệnh khớp cùng một khoản không mã vào hai thanh toán khác nhau, gửi
/// cùng lúc: chỉ một thanh toán được ghi thu — một khoản tiền không trả cho hai đơn.
#[tokio::test]
async fn hai_lenh_khop_cung_luc_mot_khoan_chi_ghi_thu_mot_lan() {
    TestDatabase::run(async |test| {
        let (app, _feed, account) = setup(test).await;
        let first = pay(&app, 50_000, "P-A").await["id"].as_str().unwrap().to_owned();
        let second = pay(&app, 50_000, "P-B").await["id"].as_str().unwrap().to_owned();
        push(&app, &[FakeBankFeed::credit("T-1", ACCOUNT, vnd(50_000), "chuyen tien")]).await;
        drain(&app).await;
        let open = exceptions(&app).await;
        assert_eq!(open.len(), 1, "{open:?}");
        let uri = format!("/exceptions/{}/resolve", open[0]["id"].as_str().unwrap());
        let (one, two) = tokio::join!(
            call(&app, "POST", &uri, Some(json!({"action": "match", "payment_id": first}))),
            call(&app, "POST", &uri, Some(json!({"action": "match", "payment_id": second}))),
        );
        assert!(one.0.is_success() || two.0.is_success(), "{one:?} {two:?}");
        let succeeded = [status(&app, &first).await, status(&app, &second).await]
            .iter()
            .filter(|status| *status == "succeeded")
            .count();
        assert_eq!(succeeded, 1, "one credit pays one payment");
        assert_eq!(bank_balance(&app, &account).await, 50_000);
    })
    .await
    .unwrap();
}
