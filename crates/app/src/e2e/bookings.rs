//! Đặt lịch trên chương trình thật (cả SQLite và Postgres): cọc thu qua `ledger` thật với cổng
//! thanh toán giả (webhook ký đúng), khách huỷ muộn thì `ledger` hoàn đúng phần theo chính sách,
//! nhắc lịch đi qua `notifications` thật và không gửi hai lần.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::sync::Arc;

use autoxide_kernel::access::Scope;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::payment::PaymentGateway;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, root_of};
use autoxide_ledger::api::Ledger;
use autoxide_testkit::{FakePaymentGateway, TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use sea_orm::{ConnectionTrait, Statement};
use serde_json::{Value, json};
use uuid::Uuid;

async fn call(
    app: &TestApp,
    method: &str,
    uri: &str,
    who: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method, &format!("/api/v1{uri}")).token(who);
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let body = if answer.body().is_empty() { Value::Null } else { answer.json() };
    (answer.status(), body)
}

async fn ok(app: &TestApp, method: &str, uri: &str, body: Value, expected: StatusCode) -> Value {
    let (status, answer) = call(app, method, uri, "owner", Some(body)).await;
    assert_eq!(status, expected, "{method} {uri}: {answer}");
    answer
}

async fn drain(app: &TestApp) {
    let worker = app.modules().worker(4).unwrap().blocked_delay(std::time::Duration::ZERO);
    let dispatcher = app.modules().dispatcher().unwrap();
    for _ in 0..10 {
        let events = dispatcher.run_once().await.unwrap();
        if worker.run_once().await.unwrap() + events == 0 {
            break;
        }
    }
}

#[tokio::test]
async fn coc_qua_ledger_huy_muon_hoan_theo_chinh_sach() {
    TestDatabase::run(async |test| {
        let gateway = Arc::new(FakePaymentGateway::new());
        let root = root_of(DEFAULT_TENANT);
        let mut builder = TestApp::builder(test)
            .member(
                "owner",
                0x0e,
                root,
                &[
                    ("bookings.manage", Scope::Tenant),
                    ("bookings.services.manage", Scope::Tenant),
                    ("organizations.settings.write", Scope::Tenant),
                    ("organizations.units.read", Scope::Tenant),
                ],
            )
            .user("ann", 0xa1, &[])
            .contribute::<dyn PaymentGateway>(gateway.clone());
        // Bài này thu qua cổng giả; bộ nối thật `momo` cũng góp `dyn PaymentGateway`, mà có hai cổng thì
        // mỗi thanh toán phải nêu đối tác, nên không nạp nó ở đây.
        for module in crate::modules::all().into_iter().filter(|module| module.name() != "momo") {
            builder = builder.boxed_module(module);
        }
        let app = builder.build().await.unwrap();

        // Chính sách của cửa hàng: huỷ miễn phí khi còn hơn 30 ngày, sau đó giữ 30% cọc.
        for (key, value) in [
            ("organizations.timezone", json!("Asia/Ho_Chi_Minh")),
            ("bookings.free_cancel_hours", json!(720)),
            ("bookings.late_cancel_retain_percent", json!(30)),
        ] {
            ok(&app, "PUT", &format!("/organizations/units/{root}/settings/{key}"), json!({"value": value}), StatusCode::OK)
                .await;
        }
        let lan = ok(&app, "POST", "/bookings/resources", json!({"kind": "staff", "name": "Lan"}), StatusCode::CREATED).await;
        let lan = lan["data"]["id"].as_str().unwrap().to_owned();
        let week = json!({"ranges": (1..=7).map(|day| json!({"weekday": day, "start_minute": 540, "end_minute": 1020})).collect::<Vec<_>>()});
        ok(&app, "PUT", &format!("/bookings/resources/{lan}/schedule"), week, StatusCode::OK).await;
        let service = json!({
            "name": "Massage", "duration_minutes": 60, "price": {"amount": "500000", "currency": "VND"},
            "deposit": {"kind": "amount", "value": 200000}, "performers": [lan],
        });
        let service = ok(&app, "POST", "/bookings/services", service, StatusCode::CREATED).await;
        let service = service["data"]["id"].as_str().unwrap().to_owned();

        // Ngày mai 10:00 giờ VN (03:00 UTC).
        let day = (Timestamp::now() + SignedDuration::from_hours(24 * 2)).date_utc();
        let start = day.start_of_day_utc() + SignedDuration::from_hours(3);
        let (status, availability) = call(
            &app,
            "GET",
            &format!(
                "/bookings/public/availability?service_id={service}&from={}&to={}",
                (start - SignedDuration::from_hours(1)).format("%Y-%m-%dT%H:%M:%SZ"),
                (start + SignedDuration::from_hours(1)).format("%Y-%m-%dT%H:%M:%SZ")
            ),
            "ann",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{availability}");
        assert_eq!(availability["data"]["terms"]["deposit"]["amount"], "200000");
        assert_eq!(availability["data"]["terms"]["late_cancel_retain_percent"], 30);

        let body = json!({"service_id": service, "starts_at": start, "customer": {"name": "Ann", "phone": "0912345678"}});
        let (status, booked) = call(&app, "POST", "/bookings/public/bookings", "ann", Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{booked}");
        assert_eq!(booked["data"]["booking"]["status"], "held");
        let id = booked["data"]["booking"]["id"].as_str().unwrap().to_owned();
        let payment: Uuid = booked["data"]["checkout"]["payment_id"].as_str().unwrap().parse().unwrap();

        // Đối tác báo đã thu (webhook ký đúng) → `ledger` phát event → lịch được xác nhận.
        let (headers, body) = gateway.paid(payment, Money::new(200_000, Currency::VND), "evt-booking-1");
        let mut request = app.request(Method::POST, "/api/v1/ledger/webhooks/fake").bytes(body);
        request = request.header("content-type", "application/json");
        for (name, value) in &headers {
            request = request.header(name, value);
        }
        assert!(request.send().await.unwrap().status().is_success());
        drain(&app).await;
        let (_, got) = call(&app, "GET", &format!("/bookings/bookings/{id}"), "ann", None).await;
        assert_eq!((got["data"]["status"].as_str(), got["data"]["deposit"]["status"].as_str()), (Some("confirmed"), Some("paid")));

        // Huỷ muộn: giữ 30% → hoàn 140.000 qua `ledger`.
        let (status, cancelled) =
            call(&app, "POST", &format!("/bookings/bookings/{id}/cancel"), "ann", Some(json!({}))).await;
        assert_eq!(status, StatusCode::OK, "{cancelled}");
        drain(&app).await;
        let ledger = app.api::<dyn Ledger>().unwrap();
        let paid = ledger.payment(&app.default_tenant(), payment).await.unwrap().unwrap();
        assert_eq!(paid.refunded.minor(), 140_000, "{paid:?}");
        let (_, got) = call(&app, "GET", &format!("/bookings/bookings/{id}"), "ann", None).await;
        assert_eq!(got["data"]["deposit"]["status"], "refunded");

        // Cửa hàng đặt hộ một khách có email, bắt đầu sau 90 phút: nhắc lần 2 giờ một lần dù job chạy hai lần.
        // Nhân viên trực cả ngày, để đặt được giờ bất kỳ.
        let night = ok(&app, "POST", "/bookings/resources", json!({"kind": "staff", "name": "Night"}), StatusCode::CREATED).await;
        let night = night["data"]["id"].as_str().unwrap().to_owned();
        let always = json!({"ranges": (1..=7).map(|day| json!({"weekday": day, "start_minute": 0, "end_minute": 1440})).collect::<Vec<_>>()});
        ok(&app, "PUT", &format!("/bookings/resources/{night}/schedule"), always, StatusCode::OK).await;
        let free = json!({"name": "Consultation", "duration_minutes": 30, "price": {"amount": "0", "currency": "VND"}, "performers": [night]});
        let free = ok(&app, "POST", "/bookings/services", free, StatusCode::CREATED).await;
        let free = free["data"]["id"].as_str().unwrap().to_owned();
        let soon = Timestamp::now() + SignedDuration::from_mins(90);
        let soon = Timestamp::from_unix_seconds(soon.as_unix_seconds().div_euclid(300) * 300).unwrap();
        let body = json!({"service_id": free, "starts_at": soon, "customer": {"name": "Binh", "email": "binh@example.com"}});
        ok(&app, "POST", "/bookings/bookings", body, StatusCode::CREATED).await;
        for _ in 0..2 {
            autoxide_kernel::job::JobQueue::new("bookings")
                .enqueue_database(app.database().write(), &RemindNow {}, autoxide_kernel::job::EnqueueOptions::default())
                .await
                .unwrap();
            drain(&app).await;
        }
        let write = app.database().read();
        let backend = write.get_database_backend();
        let rows = write
            .query_all_raw(Statement::from_string(
                backend,
                "SELECT COUNT(*) AS n FROM notifications_messages WHERE kind = 'bookings.reminder' AND channel = 'email'".to_owned(),
            ))
            .await
            .unwrap();
        let sent: i64 = rows[0].try_get("", "n").unwrap();
        assert_eq!(sent, 1, "one reminder, by email");
    })
    .await
    .unwrap();
}

#[derive(Debug, serde::Serialize)]
struct RemindNow {}

impl autoxide_kernel::job::Job for RemindNow {
    const KIND: &'static str = "bookings.send_reminders";
}
