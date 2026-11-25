//! Mục 3: nhắc lịch qua `notifications`, khách vãng lai đặt / huỷ bằng mã một lần, `/me/bookings`,
//! tệp lịch `.ics`, tool MCP.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::sync::Arc;

use autoxide_kernel::job::{EnqueueOptions, Job, JobQueue};
use autoxide_kernel::sms::SmsSender;
use autoxide_notifications::api::Notifier;
use autoxide_testkit::{FakeSmsSender, TestApp, TestDatabase, tool};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::bookings::drain;
use super::{Shop, admin, app_with, call, service, set_setting};
use autoxide_notifications::api::testing::FakeNotifier;

struct Setup {
    shop: Shop,
    sms: Arc<FakeSmsSender>,
    notifier: Arc<FakeNotifier>,
    service: String,
}

/// Cửa hàng với nhân viên trực cả tuần, dịch vụ 30 phút không cọc, SMS và thông báo giả.
async fn setup(test: &TestDatabase) -> Setup {
    let sms = Arc::new(FakeSmsSender::new());
    let notifier = Arc::new(FakeNotifier::default());
    let shop = app_with(test, |builder| {
        builder
            .user("ann", 0xa1, &[])
            .provide::<dyn SmsSender>(sms.clone())
            .provide::<dyn Notifier>(notifier.clone())
    })
    .await;
    set_setting(&shop, "organizations.timezone", json!("Asia/Ho_Chi_Minh"));
    let made = admin(
        &shop,
        "POST",
        "/bookings/resources",
        Some(json!({"kind": "staff", "name": "Night"})),
        StatusCode::CREATED,
    )
    .await;
    let night = made["data"]["id"].as_str().unwrap().to_owned();
    let always = json!({"ranges": (1..=7).map(|day| json!({"weekday": day, "start_minute": 0, "end_minute": 1440})).collect::<Vec<_>>()});
    admin(
        &shop,
        "PUT",
        &format!("/bookings/resources/{night}/schedule"),
        Some(always),
        StatusCode::OK,
    )
    .await;
    let service = service(&shop, 30, 0, json!({}), &[&night]).await;
    Setup { shop, sms, notifier, service }
}

/// Giờ trên lưới 5 phút, sau bây giờ `minutes` phút (làm tròn xuống).
fn in_minutes(minutes: i64) -> Timestamp {
    let at = Timestamp::now() + SignedDuration::from_mins(minutes);
    Timestamp::from_unix_seconds(at.as_unix_seconds().div_euclid(300) * 300).unwrap()
}

fn code_in(sms: &FakeSmsSender, phone: &str) -> String {
    let text = sms.last_text(phone).unwrap();
    text.split(|c: char| !c.is_ascii_digit()).find(|part| part.len() == 6).unwrap().to_owned()
}

/// Xin mã cho `phone` (công khai); trả `phone_code` để gửi kèm.
async fn code(setup: &Setup, phone: &str) -> Value {
    let (status, sent) = call(
        &setup.shop,
        "POST",
        "/bookings/public/phone-codes",
        None,
        Some(json!({"phone": phone})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{sent}");
    json!({"challenge_id": sent["data"]["challenge_id"], "code": code_in(&setup.sms, "+84912345678")})
}

#[derive(Debug, Serialize, Deserialize)]
struct RemindNow {}

impl Job for RemindNow {
    const KIND: &'static str = crate::reminders::SEND_REMINDERS;
}

async fn remind_now(app: &TestApp) {
    JobQueue::new("bookings")
        .enqueue_database(app.database().write(), &RemindNow {}, EnqueueOptions::default())
        .await
        .unwrap();
    drain(app).await;
}

#[tokio::test]
async fn nhac_lich_khong_gui_hai_lan() {
    TestDatabase::run(async |test| {
        let setup = setup(test).await;
        let shop = &setup.shop;
        // Lịch sau 90 phút: chỉ nhắc lần 2 giờ. Lịch sau 3 ngày: chưa nhắc.
        let soon = json!({"service_id": setup.service, "starts_at": in_minutes(90), "customer": {"name": "Ann"}});
        let (status, booked) = call(shop, "POST", "/bookings/public/bookings", Some("ann"), Some(soon)).await;
        assert_eq!(status, StatusCode::CREATED, "{booked}");
        let id = booked["data"]["booking"]["id"].as_str().unwrap().to_owned();
        let far = json!({"service_id": setup.service, "starts_at": in_minutes(3 * 24 * 60), "customer": {"name": "Ann"}});
        let (status, _) = call(shop, "POST", "/bookings/public/bookings", Some("ann"), Some(far)).await;
        assert_eq!(status, StatusCode::CREATED);

        remind_now(shop).await;
        remind_now(shop).await;
        let sent = setup.notifier.calls();
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert_eq!(sent[0].kind, "bookings.reminder");
        assert!(sent[0].dedupe_key.contains(":2h:"), "{}", sent[0].dedupe_key);
        assert_eq!(sent[0].link.as_deref(), Some(format!("/api/v1/bookings/me/bookings/{id}/calendar.ics").as_str()));

        // Đổi giờ: nhắc lại theo giờ mới, một lần.
        let moved = json!({"starts_at": in_minutes(100)});
        admin(shop, "POST", &format!("/bookings/bookings/{id}/reschedule"), Some(moved), StatusCode::OK).await;
        remind_now(shop).await;
        remind_now(shop).await;
        assert_eq!(setup.notifier.calls().len(), 2);

        // Lịch đã huỷ không được nhắc.
        let other = json!({"service_id": setup.service, "starts_at": in_minutes(60), "customer": {"name": "Ann"}});
        let (_, booked) = call(shop, "POST", "/bookings/public/bookings", Some("ann"), Some(other)).await;
        let other = booked["data"]["booking"]["id"].as_str().unwrap();
        admin(shop, "POST", &format!("/bookings/bookings/{other}/cancel"), Some(json!({})), StatusCode::OK).await;
        remind_now(shop).await;
        assert_eq!(setup.notifier.calls().len(), 2);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khach_vang_lai_dat_va_huy_bang_ma() {
    TestDatabase::run(async |test| {
        let setup = setup(test).await;
        let shop = &setup.shop;
        let start = in_minutes(48 * 60);
        let body = |phone_code: Value| {
            json!({"service_id": setup.service, "starts_at": start, "phone_code": phone_code,
                   "customer": {"name": "Guest", "phone": "0912345678"}})
        };
        // Không mã: không đặt được.
        let (status, problem) = call(shop, "POST", "/bookings/public/bookings", None, Some(json!({
            "service_id": setup.service, "starts_at": start, "customer": {"name": "Guest", "phone": "0912345678"}}))).await;
        assert_eq!((status, problem["code"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("bookings.code_invalid")), "{problem}");
        // Mã sai.
        let mut wrong = code(&setup, "0912345678").await;
        wrong["code"] = json!("000000");
        if code_in(&setup.sms, "+84912345678") == "000000" {
            wrong["code"] = json!("111111");
        }
        let (status, problem) = call(shop, "POST", "/bookings/public/bookings", None, Some(body(wrong))).await;
        assert_eq!((status, problem["code"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("bookings.code_invalid")));

        let right = code(&setup, "0912 345 678").await;
        let (status, booked) = call(shop, "POST", "/bookings/public/bookings", None, Some(body(right.clone()))).await;
        assert_eq!(status, StatusCode::CREATED, "{booked}");
        let booking = &booked["data"]["booking"];
        assert_eq!((booking["customer"]["phone"].as_str(), booking["customer"]["user_id"].as_str()), (Some("+84912345678"), None));
        let id = booking["id"].as_str().unwrap().to_owned();
        // Mã đã dùng.
        let (status, _) = call(shop, "POST", "/bookings/public/bookings", None, Some(body(right))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

        // Huỷ bằng mã mới của đúng số.
        let cancel = json!({"phone_code": code(&setup, "+84912345678").await, "reason": "Sick"});
        let (status, cancelled) =
            call(shop, "POST", &format!("/bookings/public/bookings/{id}/cancel"), None, Some(cancel)).await;
        assert_eq!((status, cancelled["data"]["status"].as_str()), (StatusCode::OK, Some("cancelled")), "{cancelled}");

        // Quá 5 mã một giờ cho cùng số.
        for _ in 0..2 {
            let (status, _) = call(shop, "POST", "/bookings/public/phone-codes", None, Some(json!({"phone": "0912345678"}))).await;
            assert_eq!(status, StatusCode::OK);
        }
        let (status, problem) = call(shop, "POST", "/bookings/public/phone-codes", None, Some(json!({"phone": "0912345678"}))).await;
        assert_eq!((status, problem["code"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("bookings.too_many_codes")));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn lich_cua_toi_tep_lich_va_tool_mcp() {
    TestDatabase::run(async |test| {
        let setup = setup(test).await;
        let shop = &setup.shop;
        let start = in_minutes(48 * 60);
        let body = json!({"service_id": setup.service, "starts_at": start, "customer": {"name": "Ann"}});
        let (_, booked) = call(shop, "POST", "/bookings/public/bookings", Some("ann"), Some(body)).await;
        let id = booked["data"]["booking"]["id"].as_str().unwrap().to_owned();

        let (status, mine) = call(shop, "GET", "/bookings/me/bookings", Some("ann"), None).await;
        assert_eq!((status, mine["data"].as_array().unwrap().len()), (StatusCode::OK, 1));
        let (status, mine) = call(shop, "GET", "/bookings/me/bookings", Some("admin"), None).await;
        assert_eq!((status, mine["data"].as_array().unwrap().len()), (StatusCode::OK, 0));
        let answer = shop.get(&format!("/api/v1/bookings/me/bookings/{id}/calendar.ics")).token("ann").send().await.unwrap();
        assert_eq!(answer.status(), StatusCode::OK);
        assert!(answer.header("content-type").starts_with("text/calendar"));
        assert!(answer.text().contains(&format!("DTSTART:{}", start.format("%Y%m%dT%H%M%SZ"))));
        let answer = shop.get(&format!("/api/v1/bookings/me/bookings/{id}/calendar.ics")).token("admin").send().await.unwrap();
        assert_eq!(answer.status(), StatusCode::NOT_FOUND);

        // MCP: cùng service với REST.
        let client = shop.mcp("admin").await.unwrap();
        let from = (start - SignedDuration::from_hours(1)).to_rfc3339();
        let to = (start + SignedDuration::from_hours(1)).to_rfc3339();
        let found = client
            .call_tool(tool("bookings.availability", json!({"service_id": setup.service, "from": from, "to": to})))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        let openings = found["data"]["openings"].as_array().unwrap();
        assert!(!openings.iter().any(|o| o["starts_at"] == json!(start)), "{found}");
        let listed = client.call_tool(tool("bookings.list_bookings", json!({}))).await.unwrap().structured_content.unwrap();
        assert_eq!(listed["data"][0]["id"], id.as_str());
        let taken = client
            .call_tool(tool("bookings.create_booking", json!({"service_id": setup.service, "starts_at": start, "customer_name": "Bob", "customer_phone": "0987654321"})))
            .await
            .unwrap();
        assert_eq!(taken.is_error, Some(true));
        assert_eq!(taken.structured_content.unwrap()["code"], "bookings.slot_taken");
        let later = start + SignedDuration::from_hours(2);
        let moved = client
            .call_tool(tool("bookings.reschedule_booking", json!({"id": id, "starts_at": later})))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(moved["data"]["starts_at"], json!(later));
        let cancelled = client
            .call_tool(tool("bookings.cancel_booking", json!({"id": id})))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert_eq!(cancelled["data"]["status"], "cancelled");
        client.cancel().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn doan_ma_dong_thoi_van_bi_chan_sau_nam_lan() {
    use sea_orm::EntityTrait as _;
    TestDatabase::run(async |test| {
        let setup = setup(test).await;
        let shop = &setup.shop;
        let start = in_minutes(48 * 60);
        let body = |phone_code: Value| {
            json!({"service_id": setup.service, "starts_at": start, "phone_code": phone_code,
                   "customer": {"name": "Guest", "phone": "0912345678"}})
        };
        let right = code(&setup, "0912345678").await;
        let mut wrong = right.clone();
        wrong["code"] = json!(if right["code"] == "000000" { "111111" } else { "000000" });
        // Sáu lần đoán cùng lúc: Postgres từng cho cả sáu cùng thấy 0 lần thử, nên giới hạn 5 lần
        // không chặn được loạt đoán song song.
        let guess =
            || call(shop, "POST", "/bookings/public/bookings", None, Some(body(wrong.clone())));
        let results = tokio::join!(guess(), guess(), guess(), guess(), guess(), guess());
        for (status, _) in [results.0, results.1, results.2, results.3, results.4, results.5] {
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        }
        // Mỗi lần được so mã là một lần thử đã đếm: không lần nào quá 5.
        let challenge: uuid::Uuid = right["challenge_id"].as_str().unwrap().parse().unwrap();
        let row = crate::entity::guest_challenge::Entity::find_by_id(challenge)
            .one(shop.database().read())
            .await
            .unwrap()
            .unwrap();
        assert!(row.attempts <= 5, "{} guesses were checked", row.attempts);
        let (status, problem) =
            call(shop, "POST", "/bookings/public/bookings", None, Some(body(right))).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("bookings.code_invalid")),
            "{problem}"
        );
    })
    .await
    .unwrap();
}

/// Pentest 2026-11: một địa chỉ client xin mã cho hàng loạt số khác nhau (mỗi tin tốn tiền SMS
/// của cửa hàng) bị chặn sau 20 mã một giờ; địa chỉ khác vẫn xin được.
#[tokio::test]
async fn mot_dia_chi_khong_xin_ma_cho_hang_loat_so() {
    TestDatabase::run(async |test| {
        let sms = Arc::new(FakeSmsSender::new());
        let shop = app_with(test, |builder| {
            builder
                .setting("AUTOXIDE_HTTP_CLIENT_IP_HEADER", "X-Forwarded-For")
                .provide::<dyn SmsSender>(sms.clone())
        })
        .await;
        let ask = async |client: &str, number: u32| {
            let answer = shop
                .post("/api/v1/bookings/public/phone-codes")
                .header("x-forwarded-for", client)
                .json(json!({"phone": format!("09123{number:05}")}))
                .send()
                .await
                .unwrap();
            let body = if answer.body().is_empty() { Value::Null } else { answer.json() };
            (answer.status(), body)
        };
        for number in 0..20 {
            let (status, sent) = ask("203.0.113.7", number).await;
            assert_eq!(status, StatusCode::OK, "{sent}");
        }
        let (status, problem) = ask("203.0.113.7", 20).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{problem}");
        assert_eq!(problem["code"], "bookings.too_many_codes");
        let (status, sent) = ask("198.51.100.9", 21).await;
        assert_eq!(status, StatusCode::OK, "{sent}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khach_tu_sua_mau_sms_ma_dat_lich() {
    TestDatabase::run(async |test| {
        let sms = Arc::new(FakeSmsSender::new());
        let shop = app_with(test, |builder| {
            builder
                .user("owner", 0x0e, &["kernel.settings.manage"])
                .provide::<dyn SmsSender>(sms.clone())
        })
        .await;
        let ask = || {
            call(
                &shop,
                "POST",
                "/bookings/public/phone-codes",
                None,
                Some(json!({"phone": "0912345678"})),
            )
        };
        // Chưa đặt: câu mặc định.
        let (status, sent) = ask().await;
        assert_eq!(status, StatusCode::OK, "{sent}");
        let code = code_in(&sms, "+84912345678");
        assert_eq!(
            sms.last_text("+84912345678").unwrap(),
            format!("Your booking code is {code}. It expires in 10 minutes.")
        );
        let setting = "/kernel/settings/bookings.sms_template.guest_code";
        let (status, problem) =
            call(&shop, "PUT", setting, Some("owner"), Some(json!({"value": "Ma {otp}"}))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        assert_eq!(problem["errors"][0]["code"], "placeholder_unknown");
        let value = json!({"value": "[SPA] Ma dat lich {code} ({minutes} phut)"});
        let (status, saved) = call(&shop, "PUT", setting, Some("owner"), Some(value)).await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        let (status, sent) = ask().await;
        assert_eq!(status, StatusCode::OK, "{sent}");
        let code = code_in(&sms, "+84912345678");
        assert_eq!(
            sms.last_text("+84912345678").unwrap(),
            format!("[SPA] Ma dat lich {code} (10 phut)")
        );
    })
    .await
    .unwrap();
}
