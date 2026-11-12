//! Đăng ký công khai và xác nhận.

use std::time::Duration;

use autoxide_kernel::antispam::AntispamConfig;
use autoxide_testkit::TestDatabase;

use super::{
    count_events, events, form_token, list, subscribe, subscribers, subscriptions, token_in,
    visitor,
};
use crate::config::NewsletterConfig;
use crate::public::SubscribeRequest;
use crate::subscribers::{SubscriberChange, SubscriberStatus, SubscriptionStatus};
use crate::testing::{admin, config, service, service_with};

/// Mới, đang chờ, đã xác nhận, bị khoá, dính bẫy, quá nhiều lần: cùng `Ok(())`; chỉ "mới" và
/// "đang chờ" sinh mail xác nhận; mail không chứa chữ người đăng ký gõ.
#[tokio::test]
async fn dang_ky_moi_truong_hop_cung_ket_qua_chi_mail_khi_can() {
    TestDatabase::run(async |test| {
        let config = NewsletterConfig { per_ip_per_hour: 100, ..config() };
        let h = service(test, config).await;
        let weekly = list(&h.service, "weekly", true).await;

        // Mới: một người, một đăng ký chờ xác nhận, một mail.
        let request = SubscribeRequest {
            email: Some(" Ada@Example.com ".to_owned()),
            name: Some("Visit http://spam.example now".to_owned()),
            lists: vec!["weekly".to_owned()],
            locale: Some("vi".to_owned()),
            consent_text: Some("Tôi muốn nhận bản tin.".to_owned()),
            source: Some("footer".to_owned()),
            form_token: Some(form_token(&h.service).await),
            ..SubscribeRequest::default()
        };
        h.service.subscribe(None, &visitor("203.0.113.9"), request).await.unwrap();
        assert_eq!(subscribers(&h.database).await, 1);
        let rows = subscriptions(&h.database).await;
        assert_eq!(rows[0].status, "unconfirmed");
        let consent: serde_json::Value =
            serde_json::from_str(rows[0].consent.as_deref().unwrap()).unwrap();
        assert_eq!(consent["text"], "Tôi muốn nhận bản tin.");
        assert_eq!(consent["source"], "footer");
        assert!(consent["ip_fingerprint"].is_string(), "{consent}");
        let mails = h.mailer.to("ada@example.com");
        assert_eq!(mails.len(), 1);
        assert_eq!(mails[0].subject, "Xác nhận đăng ký nhận bản tin");
        assert!(mails[0].text.contains("Tin weekly"), "{}", mails[0].text);
        assert!(mails[0].text.contains("https://www.example.com/confirm?token="));
        assert!(!mails[0].text.contains("spam.example"), "no text typed by the subscriber");

        // Đang chờ: gửi lại mail (mới), không thêm người.
        subscribe(&h.service, "ada@example.com", &["weekly"], "203.0.113.10").await.unwrap();
        assert_eq!((subscribers(&h.database).await, h.mailer.to("ada@example.com").len()), (1, 2));

        // Xác nhận bằng token của mail mới nhất.
        let token = token_in(&h.mailer.to("ada@example.com")[1].text);
        let confirmed = h.service.confirm(&token).await.unwrap();
        assert_eq!(confirmed[0].row.key, "weekly");
        assert_eq!(subscriptions(&h.database).await[0].status, "confirmed");
        assert_eq!(
            h.service.confirm(&token).await.unwrap_err().code(),
            "newsletter.confirmation_invalid",
            "a link works once"
        );
        // Token của mail đầu (chưa dùng) vẫn dùng được nhưng không đổi gì thêm.
        let first = token_in(&h.mailer.to("ada@example.com")[0].text);
        assert_eq!(h.service.confirm(&first).await.unwrap().len(), 1);
        assert_eq!(count_events(&h.database, "newsletter.subscription_confirmed").await, 1);

        // Đã xác nhận: không mail.
        subscribe(&h.service, "ada@example.com", &["weekly"], "203.0.113.11").await.unwrap();
        assert_eq!(h.mailer.to("ada@example.com").len(), 2);

        // Bị khoá: không đổi gì, không mail.
        subscribe(&h.service, "bob@example.com", &["weekly"], "203.0.113.12").await.unwrap();
        let bob = h
            .service
            .list_subscribers(
                &admin(),
                &crate::subscribers::SubscriberFilter {
                    email: Some("bob@".to_owned()),
                    ..Default::default()
                },
                &autoxide_kernel::pagination::PageRequest::first(
                    10,
                    autoxide_kernel::pagination::Order::Desc,
                ),
            )
            .await
            .unwrap()
            .data
            .remove(0);
        let change = SubscriberChange {
            status: Some(SubscriberStatus::Blocked),
            ..SubscriberChange::default()
        };
        h.service.update_subscriber(&admin(), bob.row.id, change, None).await.unwrap();
        subscribe(&h.service, "bob@example.com", &["weekly"], "203.0.113.13").await.unwrap();
        assert_eq!(h.mailer.to("bob@example.com").len(), 1);

        // Bẫy: không lưu gì.
        let trapped = SubscribeRequest {
            email: Some("bot@example.com".to_owned()),
            lists: vec!["weekly".to_owned()],
            form_token: Some(form_token(&h.service).await),
            trap: Some("x".to_owned()),
            ..SubscribeRequest::default()
        };
        h.service.subscribe(None, &visitor("198.51.100.1"), trapped).await.unwrap();
        assert_eq!(subscribers(&h.database).await, 2);

        // Quá nhiều lần với một email (mặc định 5 / giờ): bị bỏ lặng lẽ.
        for _ in 0..6 {
            subscribe(&h.service, "carol@example.com", &["weekly"], "198.51.100.2").await.unwrap();
        }
        assert!(h.mailer.to("carol@example.com").len() <= 3, "at most 3 confirmation mails a day");

        // Lỗi thật thì rõ ràng.
        let missing = SubscribeRequest {
            email: Some("dee@example.com".to_owned()),
            lists: vec!["weekly".to_owned()],
            ..SubscribeRequest::default()
        };
        assert_eq!(
            h.service.subscribe(None, &visitor("198.51.100.3"), missing).await.unwrap_err().code(),
            "antispam.token_missing"
        );
        let wrong = SubscribeRequest {
            email: Some("not an email".to_owned()),
            lists: vec!["nope".to_owned()],
            locale: Some("fr".to_owned()),
            form_token: Some(form_token(&h.service).await),
            ..SubscribeRequest::default()
        };
        let error = h.service.subscribe(None, &visitor("198.51.100.3"), wrong).await.unwrap_err();
        let mut fields: Vec<&str> =
            error.fields().iter().map(|field| field.field.as_str()).collect();
        fields.sort_unstable();
        assert_eq!(fields, ["email", "lists[0]", "locale"]);

        // Event không mang email.
        for (name, payload) in events(&h.database).await {
            assert!(!payload.contains("@example.com"), "{name}: {payload}");
        }
        let _ = weekly;
    })
    .await
    .unwrap();
}

/// Danh sách không bật double opt-in: xác nhận ngay, không mail — trừ khi bị giữ lại (gửi quá
/// nhanh): khi đó buộc xác nhận qua mail và ghi lý do.
#[tokio::test]
async fn khong_double_opt_in_va_bi_giu_lai() {
    TestDatabase::run(async |test| {
        let h = service(test, config()).await;
        list(&h.service, "deals", false).await;
        subscribe(&h.service, "ada@example.com", &["deals"], "203.0.113.9").await.unwrap();
        assert_eq!(subscriptions(&h.database).await[0].status, "confirmed");
        assert!(h.mailer.sent().is_empty());

        let slow =
            AntispamConfig { min_fill: Duration::from_secs(60), ..AntispamConfig::default() };
        let strict = service_with(test, config(), slow).await;
        subscribe(&strict.service, "bob@example.com", &["deals"], "203.0.113.10").await.unwrap();
        let held = subscriptions(&strict.database)
            .await
            .into_iter()
            .find(|row| row.status == SubscriptionStatus::Unconfirmed.as_str())
            .unwrap();
        assert_eq!(held.hold_reason.as_deref(), Some("too_fast"));
        assert_eq!(strict.mailer.to("bob@example.com").len(), 1);
    })
    .await
    .unwrap();
}

/// Hai (năm) request đăng ký cùng email cùng lúc: một người, một đăng ký. Cùng token gửi đồng
/// thời: như một.
#[tokio::test]
async fn dang_ky_dong_thoi_cung_email() {
    TestDatabase::run(async |test| {
        let config = NewsletterConfig { per_email_per_hour: 100, per_ip_per_hour: 100, ..config() };
        let h = service(test, config).await;
        list(&h.service, "weekly", true).await;
        let mut tasks = Vec::new();
        for index in 0..5 {
            let service = h.service.clone();
            tasks.push(tokio::spawn(async move {
                subscribe(&service, "Ada@example.com", &["weekly"], &format!("203.0.113.{index}"))
                    .await
            }));
        }
        for task in tasks {
            task.await.unwrap().unwrap();
        }
        assert_eq!(subscribers(&h.database).await, 1);
        assert_eq!(subscriptions(&h.database).await.len(), 1);

        let shared = form_token(&h.service).await;
        let mut tasks = Vec::new();
        for _ in 0..4 {
            let (service, token) = (h.service.clone(), shared.clone());
            tasks.push(tokio::spawn(async move {
                let request = SubscribeRequest {
                    email: Some("bob@example.com".to_owned()),
                    lists: vec!["weekly".to_owned()],
                    form_token: Some(token),
                    ..SubscribeRequest::default()
                };
                service.subscribe(None, &visitor("198.51.100.7"), request).await
            }));
        }
        for task in tasks {
            task.await.unwrap().unwrap();
        }
        assert_eq!(subscribers(&h.database).await, 2);
        assert_eq!(h.mailer.to("bob@example.com").len(), 1, "the same token stores once");
    })
    .await
    .unwrap();
}

/// Hai lần xác nhận cùng token cùng lúc: đúng một lần thành công, một event.
#[tokio::test]
async fn xac_nhan_dong_thoi_mot_lan() {
    TestDatabase::run(async |test| {
        let h = service(test, config()).await;
        list(&h.service, "weekly", true).await;
        subscribe(&h.service, "ada@example.com", &["weekly"], "203.0.113.9").await.unwrap();
        let token = token_in(&h.mailer.to("ada@example.com")[0].text);
        let (a, b) = tokio::join!(h.service.confirm(&token), h.service.confirm(&token));
        assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
        assert_eq!(count_events(&h.database, "newsletter.subscription_confirmed").await, 1);
    })
    .await
    .unwrap();
}

/// Mail xác nhận có giới hạn theo ngày; thiếu URL trang xác nhận thì báo rõ.
#[tokio::test]
async fn gioi_han_mail_xac_nhan_va_thieu_cau_hinh() {
    TestDatabase::run(async |test| {
        let limited = NewsletterConfig { confirm_mails_per_day: 1, ..config() };
        let h = service(test, limited).await;
        list(&h.service, "weekly", true).await;
        subscribe(&h.service, "ada@example.com", &["weekly"], "203.0.113.9").await.unwrap();
        subscribe(&h.service, "ada@example.com", &["weekly"], "203.0.113.10").await.unwrap();
        assert_eq!(h.mailer.to("ada@example.com").len(), 1);

        let bare = service(test, NewsletterConfig::default()).await;
        assert_eq!(
            subscribe(&bare.service, "bob@example.com", &["weekly"], "203.0.113.11")
                .await
                .unwrap_err()
                .code(),
            "newsletter.confirm_url_missing"
        );
    })
    .await
    .unwrap();
}
