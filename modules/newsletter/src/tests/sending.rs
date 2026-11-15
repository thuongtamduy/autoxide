//! Gửi chiến dịch, huỷ một chạm, trang lựa chọn, bounce, hẹn giờ.

use autoxide_kernel::event::{Delivered, EventHandler};
use std::sync::Arc;

use autoxide_kernel::mail::Mailer;
use autoxide_kernel::pagination::{Order, PageRequest};
use autoxide_testkit::{TestApp, TestDatabase};
use serde_json::json;

use super::{
    assert_once_each, campaign, count_events, emails, import, list, next_job, run_job, send,
    subscribe, token_in, unsubscribe_token,
};
use crate::audience::bounces::{BounceKind, DeliveryFailures};
use crate::audience::subscribers::{SubscriberChange, SubscriberFilter, SubscriberStatus};
use crate::campaigns::sending::SendJob;
use crate::campaigns::{Action, CampaignStatus};
use crate::config::NewsletterConfig;
use crate::public::PreferencesChange;
use crate::testing::{admin, config, service, service_in, user};

use super::drain;

/// Mỗi người nhận đúng một mail, có đủ header huỷ một chạm, chân thư; người bị chặn, bị khoá,
/// chưa xác nhận không nhận.
#[tokio::test]
async fn gui_moi_nguoi_mot_lan_kem_header_huy_mot_cham() {
    TestDatabase::run(async |test| {
        let h = service(test, config()).await;
        let weekly = list(&h, "weekly", true).await;
        let people = emails("reader", 5);
        import(&h, weekly.row.id, &people).await;
        // Người thứ 0 bị chặn, người thứ 1 bị khoá, một người chỉ xin mà chưa xác nhận.
        h.service
            .add_suppression(&h.tenant, &admin(), &people[0], Some("asked by phone"))
            .await
            .unwrap();
        let blocked = h
            .service
            .list_subscribers(
                &h.tenant,
                &admin(),
                &crate::audience::subscribers::SubscriberFilter {
                    email: Some(people[1].clone()),
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
        h.service
            .update_subscriber(&h.tenant, &admin(), blocked.row.id, change, None)
            .await
            .unwrap();
        subscribe(&h, "pending@example.com", &["weekly"], "203.0.113.9").await.unwrap();

        let draft = campaign(&h, weekly.row.id, false).await;
        let started = send(&h, draft.row.id).await;
        assert_eq!(started.status(), CampaignStatus::Sending);
        drain(&h).await;
        let done = h.service.find_campaign(&h.tenant, draft.row.id).await.unwrap();
        assert_eq!(done.status(), CampaignStatus::Sent);
        assert_eq!((done.counts.total, done.counts.sent, done.counts.skipped), (5, 3, 2));
        assert_once_each(&h.mailer, &people[2..]);
        assert_eq!(count_events(&h.database, "newsletter.campaign_sent").await, 1);

        let mail = h.mailer.to(&people[2]).remove(0);
        let header = |name: &str| {
            mail.headers
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
                .unwrap()
        };
        assert!(
            header("List-Unsubscribe").starts_with(
                "<https://api.example.com/api/v1/newsletter/public/unsubscribe?token="
            )
        );
        assert_eq!(header("List-Unsubscribe-Post"), "List-Unsubscribe=One-Click");
        assert!(mail.text.contains("https://www.example.com/preferences?token="), "{}", mail.text);
        assert!(mail.text.contains("Example Ltd"), "{}", mail.text);
        assert!(
            mail.html
                .as_deref()
                .unwrap()
                .contains("<a href=\"https://www.example.com/preferences?token=")
        );
        assert_eq!(mail.subject, "Hello");
        assert!(mail.mail_dedupe().starts_with("newsletter.delivery:"));
        // Gửi lại (hay chạy lại) không được.
        assert_eq!(
            h.service
                .change_state(&h.tenant, &admin(), draft.row.id, Action::Send, None)
                .await
                .unwrap_err()
                .code(),
            "newsletter.campaign_invalid_transition"
        );
    })
    .await
    .unwrap();
}

trait Dedupe {
    fn mail_dedupe(&self) -> String;
}

impl Dedupe for autoxide_kernel::mail::Mail {
    fn mail_dedupe(&self) -> String {
        self.dedupe_key.clone().unwrap_or_default()
    }
}

/// Job gửi chạy lại hay chạy song song: không ai nhận hai lần, không lần gọi `Mailer` thừa.
#[tokio::test]
async fn chay_lai_va_song_song_khong_gui_trung() {
    TestDatabase::run(async |test| {
        let h = service(test, NewsletterConfig { batch_size: 10, ..config() }).await;
        let weekly = list(&h, "weekly", true).await;
        let people = emails("reader", 30);
        import(&h, weekly.row.id, &people).await;
        let draft = campaign(&h, weekly.row.id, false).await;
        let started = send(&h, draft.row.id).await;
        assert_eq!(run_job(&h, true).await.as_deref(), Some("newsletter.prepare"));
        // Chạy lại job chụp: không thêm dòng.
        let prepare = crate::campaigns::sending::PrepareJob {
            campaign_id: draft.row.id,
            run: started.row.run,
            after: None,
        };
        h.service.prepare(&h.tenant, &prepare).await.unwrap();
        assert_eq!(
            h.service.find_campaign(&h.tenant, draft.row.id).await.unwrap().counts.total,
            30
        );

        // Ba worker cùng chạy lô đầu, rồi chạy lại nó.
        let job = SendJob { campaign_id: draft.row.id, run: started.row.run, seq: 0 };
        let (a, b, c) = tokio::join!(
            h.service.send_batch(&h.tenant, &job),
            h.service.send_batch(&h.tenant, &job),
            h.service.send_batch(&h.tenant, &job)
        );
        let sent = a.unwrap().sent + b.unwrap().sent + c.unwrap().sent;
        assert!(sent >= 10, "{sent}");
        h.service.send_batch(&h.tenant, &job).await.unwrap();
        drain(&h).await;
        assert_once_each(&h.mailer, &people);
        assert_eq!(h.mailer.calls(), 30, "each delivery handed to the mailer once");
        assert_eq!(
            h.service.find_campaign(&h.tenant, draft.row.id).await.unwrap().status(),
            CampaignStatus::Sent
        );
    })
    .await
    .unwrap();
}

/// Tạm dừng giữa chừng rồi tiếp tục: gửi đúng phần còn lại, không trùng.
#[tokio::test]
async fn tam_dung_roi_tiep_tuc_gui_dung_phan_con_lai() {
    TestDatabase::run(async |test| {
        let h = service(test, NewsletterConfig { batch_size: 10, ..config() }).await;
        let weekly = list(&h, "weekly", true).await;
        let people = emails("reader", 25);
        import(&h, weekly.row.id, &people).await;
        let draft = campaign(&h, weekly.row.id, false).await;
        send(&h, draft.row.id).await;
        run_job(&h, true).await; // chụp
        run_job(&h, true).await; // lô đầu
        assert_eq!(h.mailer.calls(), 10);
        let paused = h
            .service
            .change_state(&h.tenant, &admin(), draft.row.id, Action::Pause, None)
            .await
            .unwrap();
        assert_eq!(paused.status(), CampaignStatus::Paused);
        drain(&h).await;
        assert_eq!(h.mailer.calls(), 10, "nothing more while paused");
        assert_eq!(
            h.service
                .change_state(&h.tenant, &admin(), draft.row.id, Action::Pause, None)
                .await
                .unwrap_err()
                .code(),
            "newsletter.campaign_invalid_transition"
        );
        let resumed = h
            .service
            .change_state(&h.tenant, &admin(), draft.row.id, Action::Resume, None)
            .await
            .unwrap();
        assert_eq!(resumed.counts.pending, 15);
        drain(&h).await;
        assert_once_each(&h.mailer, &people);
        let done = h.service.find_campaign(&h.tenant, draft.row.id).await.unwrap();
        assert_eq!((done.status(), done.counts.sent), (CampaignStatus::Sent, 25));
        assert_eq!(count_events(&h.database, "newsletter.campaign_resumed").await, 1);
    })
    .await
    .unwrap();
}

/// Thư quảng cáo: nhãn đầu tiêu đề, tối đa N thư mỗi 24 giờ cho một địa chỉ; thiếu địa chỉ người
/// gửi thì không gửi được.
#[tokio::test]
async fn thu_quang_cao_co_nhan_va_gioi_han() {
    TestDatabase::run(async |test| {
        let h = service(test, NewsletterConfig { advertising_max_per_day: 2, ..config() }).await;
        let deals = list(&h, "deals", true).await;
        import(&h, deals.row.id, &["ada@example.com".to_owned()]).await;
        for _ in 0..3 {
            let ad = campaign(&h, deals.row.id, true).await;
            send(&h, ad.row.id).await;
            drain(&h).await;
        }
        let mails = h.mailer.to("ada@example.com");
        assert_eq!(mails.len(), 2);
        assert_eq!(mails[0].subject, "[QC] Hello");
        let news = campaign(&h, deals.row.id, false).await;
        send(&h, news.row.id).await;
        drain(&h).await;
        assert_eq!(h.mailer.to("ada@example.com").len(), 3, "not advertising: no limit");
        let page = h
            .service
            .list_campaigns(
                &h.tenant,
                &admin(),
                &Default::default(),
                &autoxide_kernel::pagination::PageRequest::first(
                    10,
                    autoxide_kernel::pagination::Order::Asc,
                ),
            )
            .await
            .unwrap();
        assert_eq!(page.data[2].counts.skipped, 1);

        let bare = service(test, NewsletterConfig { postal_address: None, ..config() }).await;
        let ad = campaign(&bare, deals.row.id, true).await;
        assert_eq!(
            bare.service
                .change_state(&bare.tenant, &admin(), ad.row.id, Action::Send, None)
                .await
                .unwrap_err()
                .code(),
            "newsletter.postal_address_missing"
        );
        let unconfigured = service(test, NewsletterConfig::default()).await;
        let plain = campaign(&unconfigured, deals.row.id, false).await;
        assert_eq!(
            unconfigured
                .service
                .send_test(&unconfigured.tenant, &admin(), plain.row.id, "ada@example.com")
                .await
                .unwrap_err()
                .code(),
            "newsletter.preferences_url_missing"
        );
    })
    .await
    .unwrap();
}

/// Huỷ một chạm có hiệu lực ngay; token lạ cũng "được"; trang lựa chọn đọc / sửa bằng token; người
/// huỷ giữa lúc chụp và lúc gửi bị `skipped`.
#[tokio::test]
async fn huy_mot_cham_trang_lua_chon_va_huy_giua_chung() {
    TestDatabase::run(async |test| {
        let h = service(test, config()).await;
        let weekly = list(&h, "weekly", true).await;
        list(&h, "deals", false).await;
        let people = emails("reader", 2);
        import(&h, weekly.row.id, &people).await;
        let first = campaign(&h, weekly.row.id, false).await;
        send(&h, first.row.id).await;
        drain(&h).await;
        let token = unsubscribe_token(&h.mailer.to(&people[0])[0]);

        let preferences = h.service.preferences(Some(&h.tenant), &token).await.unwrap();
        assert_eq!(preferences.subscriber.email, people[0]);
        assert_eq!(preferences.lists.len(), 2, "public lists and the person's lists");
        h.service.unsubscribe(Some(&h.tenant), &token).await.unwrap();
        h.service.unsubscribe(Some(&h.tenant), &token).await.unwrap();
        h.service.unsubscribe(Some(&h.tenant), "not-a-token").await.unwrap();
        assert_eq!(count_events(&h.database, "newsletter.subscription_unsubscribed").await, 1);
        assert_eq!(
            h.service.preferences(Some(&h.tenant), "not-a-token").await.unwrap_err().code(),
            "newsletter.link_invalid"
        );

        // Nhận lại từ trang lựa chọn, cùng lúc vào thêm danh sách `deals`.
        let change = PreferencesChange {
            lists: vec![("weekly".to_owned(), true), ("deals".to_owned(), true)],
            name: Some(Some("Ada".to_owned())),
            ..PreferencesChange::default()
        };
        let after = h.service.update_preferences(Some(&h.tenant), &token, change).await.unwrap();
        assert_eq!(after.subscriber.name.as_deref(), Some("Ada"));
        assert!(
            after.lists.iter().all(|(_, status)| status.map(|s| s.as_str()) == Some("confirmed"))
        );
        let all = PreferencesChange { unsubscribe_all: true, ..PreferencesChange::default() };
        let after = h.service.update_preferences(Some(&h.tenant), &token, all).await.unwrap();
        assert!(
            after.lists.iter().all(|(_, status)| status.map(|s| s.as_str()) != Some("confirmed"))
        );

        // Người thứ hai huỷ sau khi chụp, trước khi gửi.
        let second = campaign(&h, weekly.row.id, false).await;
        send(&h, second.row.id).await;
        run_job(&h, true).await; // chụp: chỉ người thứ hai (người đầu đã huỷ)
        let token = unsubscribe_token(&h.mailer.to(&people[1])[0]);
        h.service.unsubscribe(Some(&h.tenant), &token).await.unwrap();
        drain(&h).await;
        let done = h.service.find_campaign(&h.tenant, second.row.id).await.unwrap();
        assert_eq!((done.counts.total, done.counts.skipped, done.counts.sent), (1, 1, 0));
        assert_eq!(h.mailer.to(&people[1]).len(), 1);
    })
    .await
    .unwrap();
}

/// Bounce: mail bị từ chối hẳn (event của module `mail`, giao hai lần) → lượt gửi `failed` và
/// chặn địa chỉ; báo qua endpoint: hard / khiếu nại chặn ngay, soft chặn khi tới ngưỡng.
#[tokio::test]
async fn bounce_chan_dia_chi() {
    TestDatabase::run(async |test| {
        let h = service(test, config()).await;
        let weekly = list(&h, "weekly", true).await;
        let people = emails("reader", 3);
        import(&h, weekly.row.id, &people).await;
        let draft = campaign(&h, weekly.row.id, false).await;
        send(&h, draft.row.id).await;
        drain(&h).await;
        let mail = h.mailer.to(&people[0]).remove(0);
        let failed = Delivered::new(h.tenant.clone(), "mail", autoxide_kernel::mail::DELIVERY_FAILED.to_owned(), json!({"to": format!("Reader <{}>", people[0]), "dedupe_key": mail.dedupe_key, "code": "550"}));
        let handler = DeliveryFailures(h.service.clone());
        handler.handle(&failed).await.unwrap();
        handler.handle(&failed).await.unwrap();
        let done = h.service.find_campaign(&h.tenant, draft.row.id).await.unwrap();
        assert_eq!((done.counts.sent, done.counts.failed), (2, 1));
        assert_eq!(done.counts.bounced, 1, "a hard bounce counts in `bounced` too");
        assert_eq!(count_events(&h.database, "newsletter.suppression_added").await, 1);

        let manager = user(&["manage"]);
        let soft = |email: String| {
            let h = h.clone();
            let manager = manager.clone();
            async move { h.service.report_bounce(&h.tenant, &manager, &email, BounceKind::Soft, Some("mailbox full")).await.unwrap().suppressed }
        };
        assert!(!soft(people[1].clone()).await);
        assert!(!soft(people[1].clone()).await);
        assert!(soft(people[1].clone()).await, "the third soft bounce suppresses");
        assert!(h.service.report_bounce(&h.tenant, &manager, &people[2], BounceKind::Complaint, None).await.unwrap().suppressed);
        assert_eq!(
            h.service.report_bounce(&h.tenant, &user(&["read"]), &people[2], BounceKind::Hard, None).await.unwrap_err().code(),
            "permission_denied"
        );

        // Lần gửi sau: không ai nhận.
        let next = campaign(&h, weekly.row.id, false).await;
        send(&h, next.row.id).await;
        drain(&h).await;
        let after = h.service.find_campaign(&h.tenant, next.row.id).await.unwrap();
        assert_eq!((after.counts.sent, after.counts.skipped), (0, 3));
        let page = h.service.list_suppressions(&h.tenant, &admin(), &Default::default(), &autoxide_kernel::pagination::PageRequest::first(10, autoxide_kernel::pagination::Order::Asc)).await.unwrap();
        let reasons: Vec<&str> = page.data.iter().map(|row| row.reason.as_str()).collect();
        assert_eq!(reasons, ["hard_bounce", "soft_bounce", "complaint"]);
        h.service.remove_suppression(&h.tenant, &admin(), page.data[1].id).await.unwrap();
    })
    .await
    .unwrap();
}

/// Hẹn giờ: job chụp chỉ chạy tới giờ; hẹn lại thì job cũ thôi; giờ trong quá khứ bị từ chối;
/// `If-Match` cũ bị từ chối; huỷ thì lượt chưa gửi thành `skipped`.
#[tokio::test]
async fn hen_gio_va_huy() {
    TestDatabase::run(async |test| {
        let h = service(test, config()).await;
        let weekly = list(&h, "weekly", true).await;
        import(&h, weekly.row.id, &emails("reader", 3)).await;
        let draft = campaign(&h, weekly.row.id, false).await;
        let past = chrono::Utc::now() - chrono::Duration::hours(1);
        assert_eq!(
            h.service
                .change_state(&h.tenant, &admin(), draft.row.id, Action::Schedule(past), None)
                .await
                .unwrap_err()
                .code(),
            "validation_failed"
        );
        let stale = Some(draft.row.updated_at - chrono::Duration::seconds(1));
        let later = chrono::Utc::now() + chrono::Duration::hours(1);
        assert_eq!(
            h.service
                .change_state(&h.tenant, &admin(), draft.row.id, Action::Schedule(later), stale)
                .await
                .unwrap_err()
                .code(),
            "precondition_failed"
        );
        assert_eq!(
            h.service
                .change_state(
                    &h.tenant,
                    &user(&["manage"]),
                    draft.row.id,
                    Action::Schedule(later),
                    None
                )
                .await
                .unwrap_err()
                .code(),
            "permission_denied"
        );
        let scheduled = h
            .service
            .change_state(
                &h.tenant,
                &admin(),
                draft.row.id,
                Action::Schedule(later),
                Some(draft.row.updated_at),
            )
            .await
            .unwrap();
        assert_eq!(scheduled.status(), CampaignStatus::Scheduled);
        assert!(next_job(&h.database, false).await.is_none(), "not due yet");
        // Hẹn lại: lượt mới; job cũ (tới giờ) thấy khác lượt thì thôi.
        let rescheduled = h
            .service
            .change_state(
                &h.tenant,
                &admin(),
                draft.row.id,
                Action::Schedule(later + chrono::Duration::hours(1)),
                None,
            )
            .await
            .unwrap();
        let old = crate::campaigns::sending::PrepareJob {
            campaign_id: draft.row.id,
            run: scheduled.row.run,
            after: None,
        };
        h.service.prepare(&h.tenant, &old).await.unwrap();
        assert_eq!(
            h.service.find_campaign(&h.tenant, draft.row.id).await.unwrap().status(),
            CampaignStatus::Scheduled
        );
        // Tới giờ (gọi thẳng job của lượt mới): bắt đầu gửi.
        let due = crate::campaigns::sending::PrepareJob {
            campaign_id: draft.row.id,
            run: rescheduled.row.run,
            after: None,
        };
        h.service.prepare(&h.tenant, &due).await.unwrap();
        let sending = h.service.find_campaign(&h.tenant, draft.row.id).await.unwrap();
        assert_eq!((sending.status(), sending.counts.pending), (CampaignStatus::Sending, 3));
        let cancelled = h
            .service
            .change_state(&h.tenant, &admin(), draft.row.id, Action::Cancel, None)
            .await
            .unwrap();
        assert_eq!((cancelled.status(), cancelled.counts.skipped), (CampaignStatus::Cancelled, 3));
        drain(&h).await;
        assert_eq!(h.mailer.calls(), 0);
        assert_eq!(
            h.service
                .delete_campaign(&h.tenant, &admin(), draft.row.id, None)
                .await
                .unwrap_err()
                .code(),
            "newsletter.campaign_not_deletable"
        );
    })
    .await
    .unwrap();
}

/// Người chỉ có `newsletter.manage` sửa chiến dịch đang hẹn giờ: nó về nháp, bỏ giờ hẹn, tăng
/// `run` — job đã xếp hàng của lượt cũ thôi, không gửi gì; phải hẹn / gửi lại (cần
/// `newsletter.send`). Sửa mà không đổi gì thì vẫn hẹn giờ.
#[tokio::test]
async fn sua_chien_dich_hen_gio_ve_nhap() {
    TestDatabase::run(async |test| {
        let h = service(test, config()).await;
        let weekly = list(&h, "weekly", true).await;
        import(&h, weekly.row.id, &emails("reader", 2)).await;
        let draft = campaign(&h, weekly.row.id, false).await;
        let later = chrono::Utc::now() + chrono::Duration::hours(1);
        let scheduled = h
            .service
            .change_state(&h.tenant, &admin(), draft.row.id, Action::Schedule(later), None)
            .await
            .unwrap();
        let manager = user(&["manage"]);
        let same = crate::campaigns::CampaignInput {
            subject: Some("Hello".to_owned()),
            ..crate::campaigns::CampaignInput::default()
        };
        let kept =
            h.service.update_campaign(&h.tenant, &manager, draft.row.id, same, None).await.unwrap();
        assert_eq!((kept.status(), kept.row.run), (CampaignStatus::Scheduled, scheduled.row.run));

        let rewrite = crate::campaigns::CampaignInput {
            subject: Some("Something else".to_owned()),
            advertising: Some(true),
            ..crate::campaigns::CampaignInput::default()
        };
        let changed = h
            .service
            .update_campaign(&h.tenant, &manager, draft.row.id, rewrite, None)
            .await
            .unwrap();
        assert_eq!(changed.status(), CampaignStatus::Draft);
        assert_eq!(changed.row.scheduled_at, None);
        assert_eq!(changed.row.run, scheduled.row.run + 1);
        let (_, payload) = super::events(&h.database)
            .await
            .into_iter()
            .rfind(|(name, _)| name == "newsletter.campaign_updated")
            .unwrap();
        let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(
            (payload["before"]["status"].as_str(), payload["after"]["status"].as_str()),
            (Some("scheduled"), Some("draft"))
        );

        // Job hẹn giờ của lượt cũ tới giờ: không bắt đầu gửi.
        let old = crate::campaigns::sending::PrepareJob {
            campaign_id: draft.row.id,
            run: scheduled.row.run,
            after: None,
        };
        h.service.prepare(&h.tenant, &old).await.unwrap();
        drain(&h).await;
        let current = h.service.find_campaign(&h.tenant, draft.row.id).await.unwrap();
        assert_eq!((current.status(), current.counts.total), (CampaignStatus::Draft, 0));
        assert_eq!(h.mailer.calls(), 0);
        assert_eq!(
            h.service
                .change_state(&h.tenant, &manager, draft.row.id, Action::Schedule(later), None)
                .await
                .unwrap_err()
                .code(),
            "permission_denied"
        );
    })
    .await
    .unwrap();
}

/// Hai khách trên cùng database: cùng email người đăng ký, cùng `key` danh sách, cùng địa chỉ bị
/// chặn mà không đụng nhau. Token trong link của khách kia mang khách của nó: gọi ở tên miền chính
/// (không biết khách của request) vẫn xác nhận / huỷ đúng ở khách đó. Token của khách mặc định
/// không làm gì ở khách kia; ghép ID khách kia với phần thô của khách này là token sai (bước 33).
#[tokio::test]
async fn hai_khach_cung_email_cung_key_token_mang_khach() {
    TestDatabase::run(async |test| {
        let mailer: Arc<dyn Mailer> = Arc::new(crate::testing::FakeMailer::default());
        let app = TestApp::builder(test)
            .tenant("other")
            .provide::<dyn Mailer>(mailer)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let h = service_in(test, config(), app.registry().clone()).await;
        let other = app.tenant("other").await.unwrap();
        let other_id = other.id().as_uuid().simple().to_string();
        let o = crate::testing::Harness { tenant: other, ..h.clone() };
        let ada = "ada@example.com";
        // Phần thô của token (bỏ tiền tố khách nếu có).
        let raw = |token: &str| token.rsplit('.').next().unwrap().to_owned();

        // Cùng `key` danh sách, cùng email: mỗi khách một dòng.
        let mine = list(&h, "weekly", true).await;
        let theirs = list(&o, "weekly", true).await;
        assert_ne!(mine.row.id, theirs.row.id);
        subscribe(&h, ada, &["weekly"], "203.0.113.9").await.unwrap();
        subscribe(&o, ada, &["weekly"], "203.0.113.10").await.unwrap();
        assert_eq!(super::subscribers(&h.database).await, 2);
        let mails = h.mailer.to(ada);
        assert_eq!(mails.len(), 2, "one confirmation mail per tenant");
        let (my_confirm, their_confirm) = (token_in(&mails[0].text), token_in(&mails[1].text));
        assert!(!my_confirm.contains('.'), "default-tenant tokens keep their old form");
        assert!(their_confirm.starts_with(&format!("t{other_id}.")), "{their_confirm}");

        // Token của khách mặc định gọi trong khách kia, hay phần thô của nó gắn ID khách kia:
        // token sai, và không bị tiêu mất.
        assert_eq!(
            h.service.confirm(Some(&o.tenant), &my_confirm).await.unwrap_err().code(),
            "newsletter.confirmation_invalid"
        );
        let tampered = format!("t{other_id}.{my_confirm}");
        assert_eq!(
            h.service.confirm(None, &tampered).await.unwrap_err().code(),
            "newsletter.confirmation_invalid"
        );
        let unknown = format!("t{}.{}", uuid::Uuid::now_v7().simple(), raw(&their_confirm));
        assert_eq!(
            h.service.confirm(None, &unknown).await.unwrap_err().code(),
            "newsletter.confirmation_invalid"
        );
        assert_eq!(h.service.confirm(None, &my_confirm).await.unwrap().len(), 1);
        // Token của khách kia, gọi ở tên miền chính (không biết khách): xác nhận ở khách kia.
        assert_eq!(h.service.confirm(None, &their_confirm).await.unwrap().len(), 1);

        // Mỗi khách chỉ thấy người đăng ký, danh sách của mình; cả hai đã xác nhận.
        let page = PageRequest::first(50, Order::Asc);
        for (harness, list_id) in [(&h, mine.row.id), (&o, theirs.row.id)] {
            let found = harness
                .service
                .list_subscribers(&harness.tenant, &admin(), &SubscriberFilter::default(), &page)
                .await
                .unwrap();
            assert_eq!(found.data.len(), 1);
            assert_eq!(found.data[0].subscriptions[0].row.list_id, list_id);
            assert_eq!(found.data[0].subscriptions[0].row.status, "confirmed");
            let lists = harness.service.list_lists(&harness.tenant, &admin(), &page).await.unwrap();
            assert_eq!(lists.data.len(), 1);
            assert_eq!(lists.data[0].row.id, list_id);
        }
        assert_eq!(
            o.service.get_list(&o.tenant, &admin(), mine.row.id).await.unwrap_err().code(),
            "newsletter.list_not_found"
        );

        // Cùng địa chỉ bị chặn ở hai khách: hai dòng.
        h.service.add_suppression(&h.tenant, &admin(), "spam@example.com", None).await.unwrap();
        o.service.add_suppression(&o.tenant, &admin(), "spam@example.com", None).await.unwrap();

        // Mỗi khách gửi một chiến dịch (job của khách nào chạy bằng khách đó).
        let draft = campaign(&h, mine.row.id, false).await;
        send(&h, draft.row.id).await;
        drain(&h).await;
        let draft = campaign(&o, theirs.row.id, false).await;
        send(&o, draft.row.id).await;
        drain(&o).await;
        let tokens: Vec<String> = h
            .mailer
            .to(ada)
            .iter()
            .filter(|mail| mail.dedupe_key.as_deref().is_some_and(|key| key.contains("delivery")))
            .map(unsubscribe_token)
            .collect();
        let [mine_token, their_token] = tokens.as_slice() else { panic!("{tokens:?}") };
        assert!(their_token.starts_with(&format!("t{other_id}.")), "{their_token}");
        let status = async |harness: &crate::testing::Harness| {
            let data =
                harness.service.email_data(&harness.tenant, &user(&["export"]), ada).await.unwrap();
            data.subscriber.unwrap().subscriptions[0].row.status.clone()
        };

        // Token của khách mặc định trong khách kia, hay ghép ID khách kia: không làm gì.
        o.service.unsubscribe(Some(&o.tenant), mine_token).await.unwrap();
        let tampered = format!("t{other_id}.{mine_token}");
        h.service.unsubscribe(None, &tampered).await.unwrap();
        assert_eq!(
            h.service.preferences(None, &tampered).await.unwrap_err().code(),
            "newsletter.link_invalid"
        );
        assert_eq!((status(&h).await, status(&o).await), ("confirmed".into(), "confirmed".into()));

        // Token của khách kia ở tên miền chính: trang lựa chọn đọc đúng khách, huỷ một chạm huỷ
        // thật ở khách kia; khách mặc định không đổi.
        let preferences = h.service.preferences(None, their_token).await.unwrap();
        assert_eq!(preferences.lists[0].0.row.id, theirs.row.id);
        h.service.unsubscribe(None, their_token).await.unwrap();
        assert_eq!(
            (status(&h).await, status(&o).await),
            ("confirmed".into(), "unsubscribed".into())
        );
        let progress = o.service.find_campaign(&o.tenant, draft.row.id).await.unwrap();
        assert_eq!(progress.counts.unsubscribed, 1);
        // Token cũ không tiền tố của khách mặc định vẫn dùng được ở tên miền chính.
        h.service.unsubscribe(None, mine_token).await.unwrap();
        assert_eq!(status(&h).await, "unsubscribed");
    })
    .await
    .unwrap();
}
