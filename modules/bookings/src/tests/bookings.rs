//! Đặt chỗ: không chồng giờ khi nhiều người đặt cùng lúc, lớp học không vượt sức chứa, giữ tạm khi
//! trả cọc, huỷ / đổi giờ / vắng mặt theo chính sách.

use std::sync::Arc;

use autoxide_kernel::job::{EnqueueOptions, Job, JobQueue};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use sea_orm::{ConnectionTrait, Statement};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::fakes::{FakeLedger, FakeLedgerModule};
use super::{Shop, admin, app_with, call, next_monday, service, set_setting, staff, utc};

/// Khách có tài khoản (không permission nào).
const CUSTOMERS: [(&str, u128); 5] =
    [("ann", 0xa1), ("ben", 0xb1), ("cam", 0xc1), ("dan", 0xd1), ("eve", 0xe1)];

/// Sổ cái giả và outbox của nó (để phát `ledger.payment_succeeded`).
struct Payments {
    ledger: Arc<FakeLedger>,
    outbox: Arc<std::sync::Mutex<Option<autoxide_kernel::event::Outbox>>>,
}

impl std::ops::Deref for Payments {
    type Target = FakeLedger;
    fn deref(&self) -> &FakeLedger {
        &self.ledger
    }
}

async fn shop(test: &TestDatabase) -> (Shop, Payments) {
    let ledger = Arc::new(FakeLedger::default());
    let outbox = Arc::new(std::sync::Mutex::new(None));
    let module = FakeLedgerModule { ledger: ledger.clone(), outbox: outbox.clone() };
    let shop = app_with(test, |mut builder| {
        for (token, id) in CUSTOMERS {
            builder = builder.user(token, id, &[]);
        }
        builder.module(module)
    })
    .await;
    let ledger = Payments { ledger, outbox };
    // Cửa hàng ở Việt Nam: lịch 9:00–12:00 giờ VN = 02:00–05:00 UTC.
    set_setting(&shop, "organizations.timezone", json!("Asia/Ho_Chi_Minh"));
    (shop, ledger)
}

fn at(day: NaiveDate, hour_vn: u32, minute: u32) -> DateTime<Utc> {
    utc(day, hour_vn - 7, minute)
}

/// Khách `who` đặt `service` lúc `start`.
async fn book(
    app: &TestApp,
    who: &str,
    service: &str,
    start: DateTime<Utc>,
    extra: Value,
) -> (StatusCode, Value) {
    let mut body = json!({
        "service_id": service,
        "starts_at": start,
        "customer": {"name": who, "phone": "0912345678"},
    });
    if let (Some(body), Some(extra)) = (body.as_object_mut(), extra.as_object()) {
        body.extend(extra.clone());
    }
    call(app, "POST", "/bookings/public/bookings", Some(who), Some(body)).await
}

/// Chạy hết job đang chờ (event, hoàn cọc…).
pub(super) async fn drain(app: &TestApp) {
    let worker = app.modules().worker(4).unwrap().blocked_delay(std::time::Duration::ZERO);
    let dispatcher = app.modules().dispatcher().unwrap();
    for _ in 0..10 {
        let events = dispatcher.run_once().await.unwrap();
        if worker.run_once().await.unwrap() + events == 0 {
            break;
        }
    }
}

/// Job theo lịch chạy ngay: đưa vào hàng đợi của cả database rồi chạy.
#[derive(Debug, Serialize, Deserialize)]
struct ExpireNow {}

impl Job for ExpireNow {
    const KIND: &'static str = crate::jobs::EXPIRE_HOLDS;
}

async fn expire_now(app: &TestApp) {
    JobQueue::new("bookings")
        .enqueue_database(app.database().write(), &ExpireNow {}, EnqueueOptions::default())
        .await
        .unwrap();
    drain(app).await;
}

async fn sql(app: &TestApp, statement: &str) {
    let write = app.database().write();
    let backend = write.get_database_backend();
    write.execute_raw(Statement::from_string(backend, statement.to_owned())).await.unwrap();
}

/// `ledger` báo đã thu cọc của lịch `booking` (event `ledger.payment_succeeded`), rồi giao event.
async fn paid(shop: &Shop, ledger: &Payments, booking: &str) {
    let payment = ledger
        .payments
        .lock()
        .unwrap()
        .iter()
        .find(|payment| payment.source_id.as_deref() == Some(booking))
        .cloned()
        .unwrap();
    FakeLedger::succeed(&ledger.outbox, &shop.default_tenant(), &payment).await;
    drain(shop).await;
}

#[tokio::test]
async fn hai_nguoi_dat_cung_gio_chi_mot_thanh_cong() {
    TestDatabase::run(async |test| {
        let (shop, _) = shop(test).await;
        let lan = staff(&shop, "Lan").await;
        let haircut = service(&shop, 60, 15, json!({}), &[&lan]).await;
        let day = next_monday();
        let start = at(day, 9, 0);

        // Năm khách đặt cùng giờ, cùng lúc.
        let attempts = CUSTOMERS
            .map(|(who, _)| book(&shop, who, &haircut, start, json!({"resource_id": lan})));
        let [a, b, c, d, e] = attempts;
        let results = tokio::join!(a, b, c, d, e);
        let results = [results.0, results.1, results.2, results.3, results.4];
        let won: Vec<_> =
            results.iter().filter(|(status, _)| *status == StatusCode::CREATED).collect();
        assert_eq!(won.len(), 1, "{results:?}");
        for (status, body) in results.iter().filter(|(status, _)| *status != StatusCode::CREATED) {
            assert_eq!(
                (*status, body["code"].as_str()),
                (StatusCode::CONFLICT, Some("bookings.slot_taken"))
            );
        }
        assert_eq!(won[0].1["data"]["booking"]["status"], "confirmed");

        // Giờ chồng một phần (9:30, chạm đệm 15 phút) cũng bị chặn; 10:15 thì được.
        let (status, body) = book(&shop, "ben", &haircut, at(day, 9, 30), json!({})).await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::CONFLICT, Some("bookings.slot_taken"))
        );
        let (status, body) = book(&shop, "ben", &haircut, at(day, 10, 15), json!({})).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");

        // Ngoài giờ làm.
        let (status, body) = book(&shop, "cam", &haircut, at(day, 11, 30), json!({})).await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("bookings.outside_hours"))
        );

        // "Nhân viên bất kỳ": Lan bận lúc 9:00, Mai còn trống.
        let mai = staff(&shop, "Mai").await;
        admin(
            &shop,
            "PATCH",
            &format!("/bookings/services/{haircut}"),
            Some(json!({"performers": [lan, mai]})),
            StatusCode::OK,
        )
        .await;
        let (status, body) = book(&shop, "dan", &haircut, start, json!({})).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        assert_eq!(body["data"]["booking"]["resource_id"], mai.as_str());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn lop_hoc_khong_vuot_suc_chua() {
    TestDatabase::run(async |test| {
        let (shop, _) = shop(test).await;
        let coach = staff(&shop, "Coach").await;
        let yoga = service(&shop, 60, 0, json!({"capacity": 3}), &[&coach]).await;
        let start = at(next_monday(), 9, 0);

        let attempts = CUSTOMERS.map(|(who, _)| book(&shop, who, &yoga, start, json!({})));
        let [a, b, c, d, e] = attempts;
        let results = tokio::join!(a, b, c, d, e);
        let results = [results.0, results.1, results.2, results.3, results.4];
        let won: Vec<_> =
            results.iter().filter(|(status, _)| *status == StatusCode::CREATED).collect();
        assert_eq!(won.len(), 3, "{results:?}");
        for (status, body) in results.iter().filter(|(status, _)| *status != StatusCode::CREATED) {
            assert_eq!(*status, StatusCode::CONFLICT, "{body}");
            assert!(
                ["bookings.capacity_full", "bookings.slot_taken"]
                    .contains(&body["code"].as_str().unwrap()),
                "{body}"
            );
        }

        // Giờ trống hiện 0 chỗ → không chào 9:00; một người huỷ thì có lại một chỗ.
        let (_, starts) = super::openings(&shop, &yoga, next_monday()).await;
        assert!(!starts.iter().any(|start| start.ends_with("T09:00:00+07:00")), "{starts:?}");
        let first = won[0].1["data"]["booking"]["id"].as_str().unwrap();
        admin(
            &shop,
            "POST",
            &format!("/bookings/bookings/{first}/cancel"),
            Some(json!({})),
            StatusCode::OK,
        )
        .await;
        let (body, starts) = super::openings(&shop, &yoga, next_monday()).await;
        assert!(starts.iter().any(|start| start.ends_with("T09:00:00+07:00")), "{body}");
        let mut late = None;
        for (who, _) in CUSTOMERS {
            let (status, body) = book(&shop, who, &yoga, start, json!({})).await;
            if status == StatusCode::CREATED {
                late = Some(body);
                break;
            }
        }
        assert!(late.is_some(), "a seat came back after the cancellation");
        let (status, body) = book(&shop, "eve", &yoga, start, json!({"seats": 1})).await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::CONFLICT, Some("bookings.capacity_full"))
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn lop_con_cho_nguoi_dat_dau_cung_luc_deu_duoc() {
    TestDatabase::run(async |test| {
        let (shop, _) = shop(test).await;
        let coach = staff(&shop, "Coach").await;
        let yoga = service(&shop, 60, 0, json!({"capacity": 5}), &[&coach]).await;
        let start = at(next_monday(), 9, 0);
        // Năm người cùng là người đầu tiên của buổi. Trước đây bên vướng unique khi mở buổi nhận
        // `bookings.slot_taken` dù lớp còn chỗ.
        let attempts = CUSTOMERS.map(|(who, _)| book(&shop, who, &yoga, start, json!({})));
        let [a, b, c, d, e] = attempts;
        let results = tokio::join!(a, b, c, d, e);
        let results = [results.0, results.1, results.2, results.3, results.4];
        for (status, body) in &results {
            assert_eq!(*status, StatusCode::CREATED, "{body}");
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn giu_tam_het_han_tra_cho() {
    TestDatabase::run(async |test| {
        let (shop, ledger) = shop(test).await;
        let lan = staff(&shop, "Lan").await;
        let deposit = json!({"deposit": {"kind": "percent", "value": 50}});
        let haircut = service(&shop, 60, 0, deposit, &[&lan]).await;
        let start = at(next_monday(), 9, 0);

        let (status, body) = book(&shop, "ann", &haircut, start, json!({})).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let held = &body["data"]["booking"];
        assert_eq!(held["status"], "held");
        assert_eq!(held["deposit"]["amount"]["amount"], "100000");
        assert_eq!(body["data"]["checkout"]["checkout_url"], "https://pay.example.com/checkout");
        let id = held["id"].as_str().unwrap().to_owned();
        assert_eq!(ledger.last_payment().source_id.as_deref(), Some(id.as_str()));

        // Đang giữ: người khác không đặt được.
        let (status, _) = book(&shop, "ben", &haircut, start, json!({})).await;
        assert_eq!(status, StatusCode::CONFLICT);

        // Chưa hết hạn: job không trả chỗ.
        expire_now(&shop).await;
        let (_, got) =
            call(&shop, "GET", &format!("/bookings/bookings/{id}"), Some("ann"), None).await;
        assert_eq!(got["data"]["status"], "held");

        // Hết hạn: trả chỗ, người khác đặt được.
        // (Lọc theo trạng thái: SQLite lưu UUID dạng nhị phân, không so được với chuỗi.)
        sql(
            &shop,
            "UPDATE bookings_bookings SET hold_expires_at = created_at WHERE status = 'held'",
        )
        .await;
        expire_now(&shop).await;
        expire_now(&shop).await;
        let (_, got) =
            call(&shop, "GET", &format!("/bookings/bookings/{id}"), Some("ann"), None).await;
        assert_eq!(got["data"]["status"], "expired");
        let (status, body) = book(&shop, "ben", &haircut, start, json!({})).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");

        // Tiền về muộn sau khi đã hết hạn: hoàn đủ, không giữ chỗ lại.
        let late_payment = ledger.payments.lock().unwrap()[0].clone();
        // Event đến hai lần: chỉ hoàn một lần.
        paid(&shop, &ledger, &id).await;
        paid(&shop, &ledger, &id).await;
        drain(&shop).await;
        assert_eq!(ledger.refunded(late_payment.id), 100_000);
        let (_, got) =
            call(&shop, "GET", &format!("/bookings/bookings/{id}"), Some("ann"), None).await;
        assert_eq!(
            (got["data"]["status"].as_str(), got["data"]["deposit"]["status"].as_str()),
            (Some("expired"), Some("refunded"))
        );

        // Cọc trả đúng hạn: xác nhận.
        let (_, body) = book(&shop, "cam", &haircut, at(next_monday(), 10, 0), json!({})).await;
        let paid_id = body["data"]["booking"]["id"].as_str().unwrap().to_owned();
        paid(&shop, &ledger, &paid_id).await;
        let (_, got) =
            call(&shop, "GET", &format!("/bookings/bookings/{paid_id}"), Some("cam"), None).await;
        assert_eq!(
            (got["data"]["status"].as_str(), got["data"]["deposit"]["status"].as_str()),
            (Some("confirmed"), Some("paid"))
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn huy_muon_giu_coc_theo_chinh_sach() {
    TestDatabase::run(async |test| {
        let (shop, ledger) = shop(test).await;
        // Huỷ miễn phí chỉ khi còn hơn 30 ngày (mọi lịch trong test là "muộn"); giữ 40% cọc.
        set_setting(&shop, "bookings.free_cancel_hours", json!(720));
        set_setting(&shop, "bookings.late_cancel_retain_percent", json!(40));
        set_setting(&shop, "bookings.no_show_retain_percent", json!(100));
        let lan = staff(&shop, "Lan").await;
        let deposit = json!({"deposit": {"kind": "amount", "value": 100000}});
        let haircut = service(&shop, 60, 0, deposit, &[&lan]).await;
        let day = next_monday();

        let (_, body) = book(&shop, "ann", &haircut, at(day, 9, 0), json!({})).await;
        let id = body["data"]["booking"]["id"].as_str().unwrap().to_owned();
        paid(&shop, &ledger, &id).await;
        let payment = ledger.last_payment().id;

        // Người khác không huỷ được lịch của ann (không thấy).
        let (status, _) = call(
            &shop,
            "POST",
            &format!("/bookings/bookings/{id}/cancel"),
            Some("ben"),
            Some(json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        // Khách không đổi giờ được khi đã qua hạn miễn phí.
        let move_to = json!({"starts_at": at(day, 10, 0)});
        let (status, body) = call(
            &shop,
            "POST",
            &format!("/bookings/bookings/{id}/reschedule"),
            Some("ann"),
            Some(move_to.clone()),
        )
        .await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::CONFLICT, Some("bookings.too_late_to_change"))
        );

        let (status, body) = call(
            &shop,
            "POST",
            &format!("/bookings/bookings/{id}/cancel"),
            Some("ann"),
            Some(json!({"reason": "Busy"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["data"]["deposit"]["refund"]["amount"], "60000");
        assert_eq!(body["data"]["deposit"]["status"], "refund_pending");
        drain(&shop).await;
        drain(&shop).await;
        assert_eq!(ledger.refunded(payment), 60_000, "giữ 40% của 100.000");
        let (_, got) =
            call(&shop, "GET", &format!("/bookings/bookings/{id}"), Some("ann"), None).await;
        assert_eq!(got["data"]["deposit"]["status"], "refunded");
        // Huỷ lần hai: không hoàn thêm.
        let (status, _) = call(
            &shop,
            "POST",
            &format!("/bookings/bookings/{id}/cancel"),
            Some("ann"),
            Some(json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(ledger.refunded(payment), 60_000);

        // Cửa hàng huỷ: hoàn đủ. Cửa hàng đổi giờ: được, lịch theo giờ mới.
        let (_, body) = book(&shop, "ben", &haircut, at(day, 9, 0), json!({})).await;
        let shop_id = body["data"]["booking"]["id"].as_str().unwrap().to_owned();
        paid(&shop, &ledger, &shop_id).await;
        let moved = admin(
            &shop,
            "POST",
            &format!("/bookings/bookings/{shop_id}/reschedule"),
            Some(move_to),
            StatusCode::OK,
        )
        .await;
        assert_eq!(moved["data"]["starts_at"], json!(at(day, 10, 0)));
        let (status, _) = book(&shop, "cam", &haircut, at(day, 9, 0), json!({})).await;
        assert_eq!(status, StatusCode::CREATED, "giờ cũ được trả");
        let shop_payment = ledger
            .payments
            .lock()
            .unwrap()
            .iter()
            .find(|p| p.source_id.as_deref() == Some(shop_id.as_str()))
            .unwrap()
            .id;
        admin(
            &shop,
            "POST",
            &format!("/bookings/bookings/{shop_id}/cancel"),
            Some(json!({})),
            StatusCode::OK,
        )
        .await;
        drain(&shop).await;
        assert_eq!(ledger.refunded(shop_payment), 100_000);

        // Vắng mặt: chỉ sau giờ bắt đầu; giữ 100% → không hoàn.
        let (_, body) = book(&shop, "dan", &haircut, at(day, 11, 0), json!({})).await;
        let gone = body["data"]["booking"]["id"].as_str().unwrap().to_owned();
        paid(&shop, &ledger, &gone).await;
        let (status, body) =
            call(&shop, "POST", &format!("/bookings/bookings/{gone}/no-show"), Some("admin"), None)
                .await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::CONFLICT, Some("bookings.not_started"))
        );
        sql(
            &shop,
            "UPDATE bookings_bookings SET starts_at = created_at WHERE customer_name = 'dan'",
        )
        .await;
        let marked = admin(
            &shop,
            "POST",
            &format!("/bookings/bookings/{gone}/no-show"),
            None,
            StatusCode::OK,
        )
        .await;
        assert_eq!(
            (marked["data"]["status"].as_str(), marked["data"]["deposit"]["status"].as_str()),
            (Some("no_show"), Some("retained"))
        );
        // Khách không huỷ được lịch đã bắt đầu.
        let (_, body) =
            book(&shop, "eve", &haircut, at(day + Duration::days(7), 9, 0), json!({})).await;
        let started = body["data"]["booking"]["id"].as_str().unwrap().to_owned();
        sql(
            &shop,
            "UPDATE bookings_bookings SET starts_at = created_at WHERE customer_name = 'eve'",
        )
        .await;
        let (status, body) = call(
            &shop,
            "POST",
            &format!("/bookings/bookings/{started}/cancel"),
            Some("eve"),
            Some(json!({})),
        )
        .await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::CONFLICT, Some("bookings.too_late_to_cancel"))
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cua_hang_dat_ho_va_nhan_vien_thay_lich_cua_minh() {
    TestDatabase::run(async |test| {
        let (shop, _) = shop(test).await;
        let lan = staff(&shop, "Lan").await;
        // `reader` (chỉ `bookings.staff`) là Lan.
        admin(
            &shop,
            "PATCH",
            &format!("/bookings/resources/{lan}"),
            Some(json!({"user_id": uuid::Uuid::from_u128(0x4ead)})),
            StatusCode::OK,
        )
        .await;
        let mai = staff(&shop, "Mai").await;
        let deposit = json!({"deposit": {"kind": "percent", "value": 50}});
        let haircut = service(&shop, 60, 0, deposit, &[&lan, &mai]).await;
        let day = next_monday();
        let body = json!({
            "service_id": haircut, "resource_id": lan, "starts_at": at(day, 9, 0),
            "customer": {"name": "Walk-in", "phone": "+84 912 000 111"},
        });
        let made =
            admin(&shop, "POST", "/bookings/bookings", Some(body), StatusCode::CREATED).await;
        // Cửa hàng đặt hộ: xác nhận ngay, không cọc.
        assert_eq!(made["data"]["booking"]["status"], "confirmed");
        assert_eq!(made["data"]["booking"]["deposit"]["status"], "none");
        assert_eq!(made["data"]["booking"]["customer"]["phone"], "+84912000111");
        let id = made["data"]["booking"]["id"].as_str().unwrap();
        let body = json!({
            "service_id": haircut, "resource_id": mai, "starts_at": at(day, 9, 0),
            "customer": {"name": "Other", "email": "other@example.com"},
        });
        admin(&shop, "POST", "/bookings/bookings", Some(body), StatusCode::CREATED).await;

        let (status, listed) = call(&shop, "GET", "/bookings/bookings", Some("reader"), None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(listed["data"].as_array().unwrap().len(), 1, "chỉ lịch của Lan: {listed}");
        let (status, _) =
            call(&shop, "POST", &format!("/bookings/bookings/{id}/check-in"), Some("reader"), None)
                .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = call(
            &shop,
            "POST",
            &format!("/bookings/bookings/{id}/cancel"),
            Some("reader"),
            Some(json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, body) =
            call(&shop, "POST", &format!("/bookings/bookings/{id}/complete"), Some("reader"), None)
                .await;
        assert_eq!((status, body["data"]["status"].as_str()), (StatusCode::OK, Some("completed")));
        let (status, _) =
            call(&shop, "GET", "/bookings/bookings?status=completed", Some("admin"), None).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = call(&shop, "GET", "/bookings/bookings", Some("ann"), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // Duyệt trước khi xác nhận (dịch vụ không cọc).
        set_setting(&shop, "bookings.require_approval", json!(true));
        let free = service(&shop, 30, 0, json!({}), &[&lan]).await;
        let (_, body) = book(&shop, "ann", &free, at(day, 11, 0), json!({})).await;
        assert_eq!(body["data"]["booking"]["status"], "pending");
        let pending = body["data"]["booking"]["id"].as_str().unwrap();
        let confirmed = admin(
            &shop,
            "POST",
            &format!("/bookings/bookings/{pending}/confirm"),
            None,
            StatusCode::OK,
        )
        .await;
        assert_eq!(confirmed["data"]["status"], "confirmed");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn coc_ve_dung_luc_het_han_khong_xac_nhan_tren_cho_da_nha() {
    TestDatabase::run(async |test| {
        let (shop, ledger) = shop(test).await;
        let lan = staff(&shop, "Lan").await;
        let deposit = json!({"deposit": {"kind": "percent", "value": 50}});
        let haircut = service(&shop, 60, 0, deposit, &[&lan]).await;
        let day = next_monday();
        for (hour, who) in [(9, "ann"), (10, "cam"), (11, "dan")] {
            let start = at(day, hour, 0);
            let (status, body) = book(&shop, who, &haircut, start, json!({})).await;
            assert_eq!(status, StatusCode::CREATED, "{body}");
            let id = body["data"]["booking"]["id"].as_str().unwrap().to_owned();
            sql(
                &shop,
                "UPDATE bookings_bookings SET hold_expires_at = created_at WHERE status = 'held'",
            )
            .await;
            // Tiền cọc về đúng lúc job hết hạn chạy. Thứ tự nào cũng được, nhưng lịch `confirmed`
            // thì chỗ phải còn giữ; Postgres từng ghi `expired` thành `confirmed` sau khi chỗ đã nhả.
            tokio::join!(expire_now(&shop), paid(&shop, &ledger, &id));
            drain(&shop).await;
            let (_, got) =
                call(&shop, "GET", &format!("/bookings/bookings/{id}"), Some(who), None).await;
            let (status, body) = book(&shop, "ben", &haircut, start, json!({})).await;
            match got["data"]["status"].as_str() {
                Some("confirmed") => assert_eq!(status, StatusCode::CONFLICT, "{body}"),
                Some("expired") => {
                    assert_eq!(status, StatusCode::CREATED, "{body}");
                    assert_eq!(got["data"]["deposit"]["status"], "refunded");
                }
                other => panic!("unexpected status {other:?}"),
            }
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hai_lan_doi_gio_cung_luc_khong_ro_cho() {
    TestDatabase::run(async |test| {
        let (shop, _) = shop(test).await;
        let lan = staff(&shop, "Lan").await;
        let haircut = service(&shop, 60, 0, json!({}), &[&lan]).await;
        let day = next_monday();
        let (status, body) = book(&shop, "ann", &haircut, at(day, 9, 0), json!({})).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let id = body["data"]["booking"]["id"].as_str().unwrap().to_owned();
        let path = format!("/bookings/bookings/{id}/reschedule");
        // Hai lần đổi giờ cùng lúc: Postgres từng cho cả hai cùng trả chỗ 9:00 rồi mỗi bên giữ một
        // chỗ mới — chỗ của lần thua kẹt lại mãi.
        let (first, second) = tokio::join!(
            call(&shop, "POST", &path, Some("admin"), Some(json!({"starts_at": at(day, 10, 0)}))),
            call(&shop, "POST", &path, Some("admin"), Some(json!({"starts_at": at(day, 11, 0)}))),
        );
        for (status, body) in [&first, &second] {
            assert!(matches!(*status, StatusCode::OK | StatusCode::CONFLICT), "{body}");
        }
        let (_, got) =
            call(&shop, "GET", &format!("/bookings/bookings/{id}"), Some("ann"), None).await;
        let kept: DateTime<Utc> = got["data"]["starts_at"].as_str().unwrap().parse().unwrap();
        // Mọi giờ khác giờ lịch đang giữ đều trống.
        for hour in [9, 10, 11] {
            let start = at(day, hour, 0);
            let (status, body) = book(&shop, "ben", &haircut, start, json!({})).await;
            let taken = kept == start;
            let expected = if taken { StatusCode::CONFLICT } else { StatusCode::CREATED };
            assert_eq!(status, expected, "{hour}h: {body}");
            if status == StatusCode::CREATED {
                let ben = body["data"]["booking"]["id"].as_str().unwrap();
                admin(
                    &shop,
                    "POST",
                    &format!("/bookings/bookings/{ben}/cancel"),
                    Some(json!({})),
                    StatusCode::OK,
                )
                .await;
            }
        }
    })
    .await
    .unwrap();
}
