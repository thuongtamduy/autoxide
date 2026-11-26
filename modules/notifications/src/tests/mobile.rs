//! Kênh đẩy cho ứng dụng di động (D-220, D-368), cả SQLite và Postgres: đăng ký / xem / gỡ token của
//! chính mình (token lưu mã hoá, không trả lại), token hết hiệu lực bị xoá, tin quảng cáo không gửi
//! khi chưa đồng ý, ngoài khung giờ thì hoãn — cùng luật như web push.

use super::*;

const MOBILE_PROMO: NotificationKind = NotificationKind::new(
    Category::Marketing,
    &[Channel::MobilePush],
    template("shop.app_promo", "App sale", "Sale in the app.", "App sale {percent}%"),
);

async fn register(fx: &Fixture, who: &str, token: &str) -> String {
    let device =
        json!({"platform": "ios", "token": token, "app_id": "com.example.shop", "locale": "vi"});
    let (status, created) = call(&fx.app, "POST", "/me/device-tokens", who, Some(device)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    created["data"]["id"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn token_cua_chinh_minh_va_token_het_hieu_luc_bi_xoa() {
    TestDatabase::run(async |test| {
        let fx = fixture(test).await;
        let first = register(&fx, "alice", "apns-token-1").await;
        let second = register(&fx, "alice", "apns-token-2").await;
        // Đăng ký lại cùng token: cùng dòng.
        assert_eq!(register(&fx, "alice", "apns-token-1").await, first);
        let (status, list) = call(&fx.app, "GET", "/me/device-tokens", "alice", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(list["data"].as_array().unwrap().len(), 2);
        assert!(!list.to_string().contains("apns-token"), "{list}");
        // Người khác không thấy, không gỡ được thiết bị của alice.
        let (_, other) = call(&fx.app, "GET", "/me/device-tokens", "admin", None).await;
        assert!(other["data"].as_array().unwrap().is_empty());
        let (status, _) =
            call(&fx.app, "DELETE", &format!("/me/device-tokens/{second}"), "admin", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        // Token lưu đã mã hoá.
        let stored = fx
            .app
            .database()
            .read()
            .query_all(
                &Query::select()
                    .column(Alias::new("token"))
                    .from(Alias::new("notifications_device_tokens"))
                    .to_owned(),
            )
            .await
            .unwrap();
        assert!(
            stored
                .iter()
                .all(|row| row.try_get::<String>("", "token").unwrap().starts_with("sealed:"))
        );

        // Nhà cung cấp báo token 1 hết hiệu lực: xoá; token 2 vẫn nhận.
        fx.mobile_push.mark_gone("apns-token-1");
        fx.notifier.notify(DEFAULT_TENANT, to_alice(&PUSHED, "m-1")).await.unwrap();
        run_jobs(&fx.app).await;
        assert_eq!(fx.mobile_push.count(), 1);
        assert_eq!(statuses(&fx.app, "m-1").await["mobile_push"].0, "sent");
        let (_, list) = call(&fx.app, "GET", "/me/device-tokens", "alice", None).await;
        let ids: Vec<&str> = list["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, [second.as_str()]);
        // Gỡ thiết bị cuối: lần sau kênh di động không có địa chỉ.
        let (status, _) =
            call(&fx.app, "DELETE", &format!("/me/device-tokens/{second}"), "alice", None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let outcome = fx.notifier.notify(DEFAULT_TENANT, to_alice(&PUSHED, "m-2")).await.unwrap();
        assert!(
            outcome
                .skipped
                .iter()
                .any(|skip| skip.channel == Channel::MobilePush
                    && skip.reason == SkipReason::NoAddress),
            "{outcome:?}"
        );

        // Tool MCP chỉ đọc: thiết bị của chính người gọi.
        let client = fx.app.mcp("alice").await.unwrap();
        let listed = client
            .call_tool(autoxide_testkit::tool("notifications.list_my_devices", json!({})))
            .await
            .unwrap();
        assert_ne!(listed.is_error, Some(true), "{listed:?}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tin_quang_cao_khong_gui_khi_chua_dong_y_va_hoan_ngoai_khung_gio() {
    TestDatabase::run(async |test| {
        let fx = fixture(test).await;
        fx.notifier.declare(MOBILE_PROMO).unwrap();
        register(&fx, "alice", "fcm-token-1").await;
        // Chưa đồng ý: không xếp tin cho kênh di động.
        let outcome = fx.notifier.notify(DEFAULT_TENANT, to_alice(&MOBILE_PROMO, "ad-1")).await.unwrap();
        assert!(outcome.message_ids.is_empty());
        assert_eq!(outcome.skipped[0].reason, SkipReason::NoConsent);

        // Đồng ý kênh `push`, nhưng khung giờ `push` mở sau 2 giờ nữa: hoãn.
        fx.ledger.allow(ConsentSubject::User(Uuid::from_u128(2)), ConsentKey::new("marketing", "", "push"));
        let from = (Timestamp::now() + SignedDuration::from_hours(2)).truncate(jiff::Unit::Hour);
        let window = json!({"from": from.format("%H:%M").to_string(), "to": (from + SignedDuration::from_hours(1)).format("%H:%M").to_string()});
        fx.rules.0.lock().unwrap().insert(
            "notifications.send_windows".to_owned(),
            json!({"utc_offset_minutes": 0, "marketing": {"push": window}}),
        );
        fx.notifier.notify(DEFAULT_TENANT, to_alice(&MOBILE_PROMO, "ad-2")).await.unwrap();
        run_jobs(&fx.app).await;
        assert_eq!(fx.mobile_push.count(), 0);
        assert_eq!(statuses(&fx.app, "ad-2").await["mobile_push"].0, "deferred");

        // Rút đồng ý trước giờ mở khung: kiểm lại ngay trước khi gửi → bỏ.
        fx.ledger.forget(&ConsentSubject::User(Uuid::from_u128(2)));
        fx.rules.0.lock().unwrap().insert(
            "notifications.send_windows".to_owned(),
            json!({"utc_offset_minutes": 0, "marketing": {"push": null}}),
        );
        fx.app
            .database()
            .write()
            .execute(
                &Query::update()
                    .table(Alias::new("notifications_jobs"))
                    .value(Alias::new("run_at"), Timestamp::now() - SignedDuration::from_mins(1))
                    .and_where(Expr::col(Alias::new("completed_at")).is_null())
                    .to_owned(),
            )
            .await
            .unwrap();
        run_jobs(&fx.app).await;
        assert_eq!(fx.mobile_push.count(), 0);
        assert_eq!(
            statuses(&fx.app, "ad-2").await["mobile_push"],
            ("skipped".to_owned(), "consent_withdrawn".to_owned())
        );

        // Đồng ý lại, trong khung: gửi.
        fx.ledger.allow(ConsentSubject::User(Uuid::from_u128(2)), ConsentKey::new("marketing", "", "push"));
        fx.notifier.notify(DEFAULT_TENANT, to_alice(&MOBILE_PROMO, "ad-3")).await.unwrap();
        run_jobs(&fx.app).await;
        assert_eq!(fx.mobile_push.count(), 1);
        assert_eq!(fx.mobile_push.sent.lock().unwrap()[0].1.body, "App sale 30%");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dang_ky_cung_token_dong_thoi_van_gui_duoc() {
    TestDatabase::run(async |test| {
        let fx = fixture(test).await;
        // Ứng dụng đăng ký lại token hai lần cùng lúc (mở app, làm mới token). Postgres từng cho cả
        // hai cùng thấy "chưa có": bản mã gắn ID của bên thua nằm trong dòng mang ID bên thắng, gửi
        // tin không giải mã được, và bên thua nhận 500.
        let (first, second) = tokio::join!(
            register(&fx, "alice", "apns-token-race"),
            register(&fx, "alice", "apns-token-race")
        );
        assert_eq!(first, second);
        fx.notifier.notify(DEFAULT_TENANT, to_alice(&PUSHED, "m-race")).await.unwrap();
        run_jobs(&fx.app).await;
        assert_eq!(statuses(&fx.app, "m-race").await["mobile_push"].0, "sent");
        assert_eq!(fx.mobile_push.count(), 1);
    })
    .await
    .unwrap();
}

// Duyệt hết danh sách `first` (đã có `?limit=…`) theo `next_cursor`; trả trường `field` theo thứ tự nhận.
async fn walk(fx: &Fixture, first: &str, field: &str) -> Vec<String> {
    let mut seen = Vec::new();
    let mut uri = first.to_owned();
    loop {
        let (status, page) = call(&fx.app, "GET", &uri, "alice", None).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {page}");
        let rows = page["data"].as_array().unwrap();
        assert!(rows.len() <= 2, "{page}");
        seen.extend(rows.iter().map(|row| row[field].as_str().unwrap().to_owned()));
        match page["pagination"]["next_cursor"].as_str() {
            Some(cursor) => uri = format!("{first}&cursor={cursor}"),
            None => return seen,
        }
    }
}

/// Bước 70 P9 (D-20261125-list-pages): thiết bị (mới trước), loại thông báo và tuỳ chọn (theo khoá loại) là
/// một trang; theo `next_cursor` tới hết không trùng không sót, hai chiều; tool `notifications.list_my_devices`
/// và `notifications.list_kinds` cùng tham số trả đúng trang REST trả.
#[tokio::test]
async fn thiet_bi_loai_va_tuy_chon_phan_trang() {
    TestDatabase::run(async |test| {
        let fx = fixture(test).await;
        let mut devices = Vec::new();
        for index in 0..5 {
            devices.push(register(&fx, "alice", &format!("apns-token-page-{index}")).await);
        }
        assert_eq!(walk(&fx, "/me/device-tokens?limit=2&order=asc", "id").await, devices);
        let newest_first: Vec<String> = devices.iter().rev().cloned().collect();
        assert_eq!(walk(&fx, "/me/device-tokens?limit=2", "id").await, newest_first);

        let kinds = walk(&fx, "/kinds?limit=2", "key").await;
        let mut sorted = kinds.clone();
        sorted.sort();
        sorted.dedup();
        assert!(kinds.len() > 2);
        assert_eq!(kinds, sorted);
        let down = walk(&fx, "/kinds?limit=2&order=desc", "key").await;
        assert_eq!(down, kinds.iter().rev().cloned().collect::<Vec<_>>());
        assert_eq!(walk(&fx, "/me/preferences?limit=2", "kind").await, kinds);

        let client = fx.app.mcp("alice").await.unwrap();
        for (tool, uri, arguments) in [
            ("notifications.list_my_devices", "/me/device-tokens?limit=2", json!({"limit": 2})),
            (
                "notifications.list_kinds",
                "/kinds?limit=2&order=desc",
                json!({"limit": 2, "order": "desc"}),
            ),
        ] {
            let (_, rest) = call(&fx.app, "GET", uri, "alice", None).await;
            let page =
                client.call_tool(autoxide_testkit::tool(tool, arguments.clone())).await.unwrap();
            let page = page.structured_content.unwrap();
            assert_eq!(page["data"], rest["data"], "{tool}");
            assert_eq!(page["pagination"], rest["pagination"], "{tool}");
            let mut arguments = arguments;
            arguments["cursor"] = page["pagination"]["next_cursor"].clone();
            let next = client.call_tool(autoxide_testkit::tool(tool, arguments)).await.unwrap();
            let next = next.structured_content.unwrap();
            let cursor = page["pagination"]["next_cursor"].as_str().unwrap();
            let (_, rest_next) =
                call(&fx.app, "GET", &format!("{uri}&cursor={cursor}"), "alice", None).await;
            assert_eq!(next["data"], rest_next["data"], "{tool}");
        }
        client.cancel().await.unwrap();
    })
    .await
    .unwrap();
}
