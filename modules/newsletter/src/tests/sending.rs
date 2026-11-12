//! Gửi chiến dịch, huỷ một chạm, trang lựa chọn, bounce, hẹn giờ.

use autoxide_kernel::event::{Actor, Delivered, EventHandler};
use autoxide_testkit::TestDatabase;
use serde_json::json;
use uuid::Uuid;

use super::{
    assert_once_each, campaign, count_events, emails, import, list, next_job, run_job, send,
    subscribe, unsubscribe_token,
};
use crate::bounces::{BounceKind, DeliveryFailures};
use crate::campaigns::{Action, CampaignStatus};
use crate::config::NewsletterConfig;
use crate::public::PreferencesChange;
use crate::sending::SendJob;
use crate::subscribers::{SubscriberChange, SubscriberStatus};
use crate::testing::{admin, config, service, user};

use super::drain;

/// Mỗi người nhận đúng một mail, có đủ header huỷ một chạm, chân thư; người bị chặn, bị khoá,
/// chưa xác nhận không nhận.
#[tokio::test]
async fn gui_moi_nguoi_mot_lan_kem_header_huy_mot_cham() {
    TestDatabase::run(async |test| {
        let h = service(test, config()).await;
        let weekly = list(&h.service, "weekly", true).await;
        let people = emails("reader", 5);
        import(&h.service, weekly.row.id, &people).await;
        // Người thứ 0 bị chặn, người thứ 1 bị khoá, một người chỉ xin mà chưa xác nhận.
        h.service.add_suppression(&admin(), &people[0], Some("asked by phone")).await.unwrap();
        let blocked = h
            .service
            .list_subscribers(
                &admin(),
                &crate::subscribers::SubscriberFilter {
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
        h.service.update_subscriber(&admin(), blocked.row.id, change, None).await.unwrap();
        subscribe(&h.service, "pending@example.com", &["weekly"], "203.0.113.9").await.unwrap();

        let draft = campaign(&h.service, weekly.row.id, false).await;
        let started = send(&h.service, draft.row.id).await;
        assert_eq!(started.status(), CampaignStatus::Sending);
        drain(&h).await;
        let done = h.service.find_campaign(draft.row.id).await.unwrap();
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
                .change_state(&admin(), draft.row.id, Action::Send, None)
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
        let weekly = list(&h.service, "weekly", true).await;
        let people = emails("reader", 30);
        import(&h.service, weekly.row.id, &people).await;
        let draft = campaign(&h.service, weekly.row.id, false).await;
        let started = send(&h.service, draft.row.id).await;
        assert_eq!(run_job(&h, true).await.as_deref(), Some("newsletter.prepare"));
        // Chạy lại job chụp: không thêm dòng.
        let prepare = crate::sending::PrepareJob {
            campaign_id: draft.row.id,
            run: started.row.run,
            after: None,
        };
        h.service.prepare(&prepare).await.unwrap();
        assert_eq!(h.service.find_campaign(draft.row.id).await.unwrap().counts.total, 30);

        // Ba worker cùng chạy lô đầu, rồi chạy lại nó.
        let job = SendJob { campaign_id: draft.row.id, run: started.row.run, seq: 0 };
        let (a, b, c) = tokio::join!(
            h.service.send_batch(&job),
            h.service.send_batch(&job),
            h.service.send_batch(&job)
        );
        let sent = a.unwrap().sent + b.unwrap().sent + c.unwrap().sent;
        assert!(sent >= 10, "{sent}");
        h.service.send_batch(&job).await.unwrap();
        drain(&h).await;
        assert_once_each(&h.mailer, &people);
        assert_eq!(h.mailer.calls(), 30, "each delivery handed to the mailer once");
        assert_eq!(
            h.service.find_campaign(draft.row.id).await.unwrap().status(),
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
        let weekly = list(&h.service, "weekly", true).await;
        let people = emails("reader", 25);
        import(&h.service, weekly.row.id, &people).await;
        let draft = campaign(&h.service, weekly.row.id, false).await;
        send(&h.service, draft.row.id).await;
        run_job(&h, true).await; // chụp
        run_job(&h, true).await; // lô đầu
        assert_eq!(h.mailer.calls(), 10);
        let paused =
            h.service.change_state(&admin(), draft.row.id, Action::Pause, None).await.unwrap();
        assert_eq!(paused.status(), CampaignStatus::Paused);
        drain(&h).await;
        assert_eq!(h.mailer.calls(), 10, "nothing more while paused");
        assert_eq!(
            h.service
                .change_state(&admin(), draft.row.id, Action::Pause, None)
                .await
                .unwrap_err()
                .code(),
            "newsletter.campaign_invalid_transition"
        );
        let resumed =
            h.service.change_state(&admin(), draft.row.id, Action::Resume, None).await.unwrap();
        assert_eq!(resumed.counts.pending, 15);
        drain(&h).await;
        assert_once_each(&h.mailer, &people);
        let done = h.service.find_campaign(draft.row.id).await.unwrap();
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
        let deals = list(&h.service, "deals", true).await;
        import(&h.service, deals.row.id, &["ada@example.com".to_owned()]).await;
        for _ in 0..3 {
            let ad = campaign(&h.service, deals.row.id, true).await;
            send(&h.service, ad.row.id).await;
            drain(&h).await;
        }
        let mails = h.mailer.to("ada@example.com");
        assert_eq!(mails.len(), 2);
        assert_eq!(mails[0].subject, "[QC] Hello");
        let news = campaign(&h.service, deals.row.id, false).await;
        send(&h.service, news.row.id).await;
        drain(&h).await;
        assert_eq!(h.mailer.to("ada@example.com").len(), 3, "not advertising: no limit");
        let page = h
            .service
            .list_campaigns(
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
        let ad = campaign(&bare.service, deals.row.id, true).await;
        assert_eq!(
            bare.service
                .change_state(&admin(), ad.row.id, Action::Send, None)
                .await
                .unwrap_err()
                .code(),
            "newsletter.postal_address_missing"
        );
        let unconfigured = service(test, NewsletterConfig::default()).await;
        let plain = campaign(&unconfigured.service, deals.row.id, false).await;
        assert_eq!(
            unconfigured
                .service
                .send_test(&admin(), plain.row.id, "ada@example.com")
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
        let weekly = list(&h.service, "weekly", true).await;
        list(&h.service, "deals", false).await;
        let people = emails("reader", 2);
        import(&h.service, weekly.row.id, &people).await;
        let first = campaign(&h.service, weekly.row.id, false).await;
        send(&h.service, first.row.id).await;
        drain(&h).await;
        let token = unsubscribe_token(&h.mailer.to(&people[0])[0]);

        let preferences = h.service.preferences(&token).await.unwrap();
        assert_eq!(preferences.subscriber.email, people[0]);
        assert_eq!(preferences.lists.len(), 2, "public lists and the person's lists");
        h.service.unsubscribe(&token).await.unwrap();
        h.service.unsubscribe(&token).await.unwrap();
        h.service.unsubscribe("not-a-token").await.unwrap();
        assert_eq!(count_events(&h.database, "newsletter.subscription_unsubscribed").await, 1);
        assert_eq!(
            h.service.preferences("not-a-token").await.unwrap_err().code(),
            "newsletter.link_invalid"
        );

        // Nhận lại từ trang lựa chọn, cùng lúc vào thêm danh sách `deals`.
        let change = PreferencesChange {
            lists: vec![("weekly".to_owned(), true), ("deals".to_owned(), true)],
            name: Some(Some("Ada".to_owned())),
            ..PreferencesChange::default()
        };
        let after = h.service.update_preferences(&token, change).await.unwrap();
        assert_eq!(after.subscriber.name.as_deref(), Some("Ada"));
        assert!(
            after.lists.iter().all(|(_, status)| status.map(|s| s.as_str()) == Some("confirmed"))
        );
        let all = PreferencesChange { unsubscribe_all: true, ..PreferencesChange::default() };
        let after = h.service.update_preferences(&token, all).await.unwrap();
        assert!(
            after.lists.iter().all(|(_, status)| status.map(|s| s.as_str()) != Some("confirmed"))
        );

        // Người thứ hai huỷ sau khi chụp, trước khi gửi.
        let second = campaign(&h.service, weekly.row.id, false).await;
        send(&h.service, second.row.id).await;
        run_job(&h, true).await; // chụp: chỉ người thứ hai (người đầu đã huỷ)
        let token = unsubscribe_token(&h.mailer.to(&people[1])[0]);
        h.service.unsubscribe(&token).await.unwrap();
        drain(&h).await;
        let done = h.service.find_campaign(second.row.id).await.unwrap();
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
        let weekly = list(&h.service, "weekly", true).await;
        let people = emails("reader", 3);
        import(&h.service, weekly.row.id, &people).await;
        let draft = campaign(&h.service, weekly.row.id, false).await;
        send(&h.service, draft.row.id).await;
        drain(&h).await;
        let mail = h.mailer.to(&people[0]).remove(0);
        let failed = Delivered {
            id: Uuid::now_v7(),
            module: "mail",
            name: autoxide_kernel::mail::DELIVERY_FAILED.to_owned(),
            actor: Actor::system(),
            subject: None,
            payload: json!({"to": format!("Reader <{}>", people[0]), "dedupe_key": mail.dedupe_key, "code": "550"}),
            occurred_at: chrono::Utc::now(),
        };
        let handler = DeliveryFailures(h.service.clone());
        handler.handle(&failed).await.unwrap();
        handler.handle(&failed).await.unwrap();
        let done = h.service.find_campaign(draft.row.id).await.unwrap();
        assert_eq!((done.counts.sent, done.counts.failed), (2, 1));
        assert_eq!(count_events(&h.database, "newsletter.suppression_added").await, 1);

        let manager = user(&["manage"]);
        let soft = |email: String| {
            let service = h.service.clone();
            let manager = manager.clone();
            async move { service.report_bounce(&manager, &email, BounceKind::Soft, Some("mailbox full")).await.unwrap().suppressed }
        };
        assert!(!soft(people[1].clone()).await);
        assert!(!soft(people[1].clone()).await);
        assert!(soft(people[1].clone()).await, "the third soft bounce suppresses");
        assert!(h.service.report_bounce(&manager, &people[2], BounceKind::Complaint, None).await.unwrap().suppressed);
        assert_eq!(
            h.service.report_bounce(&user(&["read"]), &people[2], BounceKind::Hard, None).await.unwrap_err().code(),
            "permission_denied"
        );

        // Lần gửi sau: không ai nhận.
        let next = campaign(&h.service, weekly.row.id, false).await;
        send(&h.service, next.row.id).await;
        drain(&h).await;
        let after = h.service.find_campaign(next.row.id).await.unwrap();
        assert_eq!((after.counts.sent, after.counts.skipped), (0, 3));
        let page = h.service.list_suppressions(&admin(), &Default::default(), &autoxide_kernel::pagination::PageRequest::first(10, autoxide_kernel::pagination::Order::Asc)).await.unwrap();
        let reasons: Vec<&str> = page.data.iter().map(|row| row.reason.as_str()).collect();
        assert_eq!(reasons, ["hard_bounce", "soft_bounce", "complaint"]);
        h.service.remove_suppression(&admin(), page.data[1].id).await.unwrap();
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
        let weekly = list(&h.service, "weekly", true).await;
        import(&h.service, weekly.row.id, &emails("reader", 3)).await;
        let draft = campaign(&h.service, weekly.row.id, false).await;
        let past = chrono::Utc::now() - chrono::Duration::hours(1);
        assert_eq!(
            h.service
                .change_state(&admin(), draft.row.id, Action::Schedule(past), None)
                .await
                .unwrap_err()
                .code(),
            "validation_failed"
        );
        let stale = Some(draft.row.updated_at - chrono::Duration::seconds(1));
        let later = chrono::Utc::now() + chrono::Duration::hours(1);
        assert_eq!(
            h.service
                .change_state(&admin(), draft.row.id, Action::Schedule(later), stale)
                .await
                .unwrap_err()
                .code(),
            "precondition_failed"
        );
        assert_eq!(
            h.service
                .change_state(&user(&["manage"]), draft.row.id, Action::Schedule(later), None)
                .await
                .unwrap_err()
                .code(),
            "permission_denied"
        );
        let scheduled = h
            .service
            .change_state(
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
                &admin(),
                draft.row.id,
                Action::Schedule(later + chrono::Duration::hours(1)),
                None,
            )
            .await
            .unwrap();
        let old = crate::sending::PrepareJob {
            campaign_id: draft.row.id,
            run: scheduled.row.run,
            after: None,
        };
        h.service.prepare(&old).await.unwrap();
        assert_eq!(
            h.service.find_campaign(draft.row.id).await.unwrap().status(),
            CampaignStatus::Scheduled
        );
        // Tới giờ (gọi thẳng job của lượt mới): bắt đầu gửi.
        let due = crate::sending::PrepareJob {
            campaign_id: draft.row.id,
            run: rescheduled.row.run,
            after: None,
        };
        h.service.prepare(&due).await.unwrap();
        let sending = h.service.find_campaign(draft.row.id).await.unwrap();
        assert_eq!((sending.status(), sending.counts.pending), (CampaignStatus::Sending, 3));
        let cancelled =
            h.service.change_state(&admin(), draft.row.id, Action::Cancel, None).await.unwrap();
        assert_eq!((cancelled.status(), cancelled.counts.skipped), (CampaignStatus::Cancelled, 3));
        drain(&h).await;
        assert_eq!(h.mailer.calls(), 0);
        assert_eq!(
            h.service.delete_campaign(&admin(), draft.row.id, None).await.unwrap_err().code(),
            "newsletter.campaign_not_deletable"
        );
    })
    .await
    .unwrap();
}
