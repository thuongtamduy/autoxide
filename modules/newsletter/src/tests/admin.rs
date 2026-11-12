//! Danh sách, nhập, xuất / xoá theo email, dọn dẹp, bài mới, tài khoản bị xoá.

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Delivered, EventHandler};
use autoxide_kernel::pagination::{Order, PageRequest};
use autoxide_kernel::privacy::{PersonalData, USER_DELETED};
use autoxide_kernel::resource::{Resource, ResourceResolver};
use autoxide_testkit::TestDatabase;
use serde_json::json;
use uuid::Uuid;

use super::{
    campaign, count_events, drain, emails, events, import, list, subscribe, subscribers,
    subscriptions,
};
use crate::campaigns::{Action, CampaignFilter, CampaignStatus};
use crate::lists::{ListInput, NewPosts};
use crate::new_posts::{ENTRY_PUBLISHED, NewPostHandler};
use crate::subscribers::{ImportInput, ImportPerson, SubscriberChange};
use crate::testing::{admin, config, service, user};
use crate::text::{Locale, Localized};

fn page() -> PageRequest {
    PageRequest::first(50, Order::Asc)
}

#[tokio::test]
async fn danh_sach_tao_sua_xoa() {
    TestDatabase::run(async |test| {
        let h = service(test, config()).await;
        let weekly = list(&h.service, "weekly", true).await;
        assert!(weekly.row.public && weekly.row.double_opt_in);
        assert_eq!(weekly.new_posts(), NewPosts::Off);
        let again = ListInput {
            key: Some("weekly".to_owned()),
            name: Some(Localized { en: Some("Again".to_owned()), vi: None }),
            ..ListInput::default()
        };
        assert_eq!(
            h.service.create_list(&admin(), again).await.unwrap_err().code(),
            "newsletter.list_key_taken"
        );
        let bad = ListInput {
            key: Some("Bad Key".to_owned()),
            name: Some(Localized::default()),
            ..ListInput::default()
        };
        let error = h.service.create_list(&admin(), bad).await.unwrap_err();
        assert_eq!(error.fields().len(), 2, "{error:?}");
        assert_eq!(
            h.service.create_list(&user(&["read"]), ListInput::default()).await.unwrap_err().code(),
            "permission_denied"
        );

        let change = ListInput {
            new_posts: Some(NewPosts::Draft),
            new_post_locale: Some(Some(Locale::Vi)),
            ..ListInput::default()
        };
        let changed = h
            .service
            .update_list(&admin(), weekly.row.id, change, Some(weekly.row.updated_at))
            .await
            .unwrap();
        assert_eq!(
            (changed.new_posts(), changed.new_post_locale()),
            (NewPosts::Draft, Some(Locale::Vi))
        );
        assert_eq!(
            h.service
                .update_list(
                    &admin(),
                    weekly.row.id,
                    ListInput::default(),
                    Some(weekly.row.updated_at)
                )
                .await
                .unwrap_err()
                .code(),
            "precondition_failed"
        );
        import(&h.service, weekly.row.id, &emails("reader", 2)).await;
        let counts = h.service.subscription_counts(&[weekly.row.id]).await.unwrap();
        assert_eq!(counts[&weekly.row.id].confirmed, 2);

        // Còn chiến dịch nháp: không xoá được; xoá nháp rồi xoá danh sách (kèm đăng ký).
        let draft = campaign(&h.service, weekly.row.id, false).await;
        assert_eq!(
            h.service.delete_list(&admin(), weekly.row.id, None).await.unwrap_err().code(),
            "newsletter.list_in_use"
        );
        h.service
            .delete_campaign(&admin(), draft.row.id, Some(draft.row.updated_at))
            .await
            .unwrap();
        h.service.delete_list(&admin(), weekly.row.id, None).await.unwrap();
        assert!(subscriptions(&h.database).await.is_empty());
        assert_eq!(
            h.service.get_list(&admin(), weekly.row.id).await.unwrap_err().code(),
            "newsletter.list_not_found"
        );
    })
    .await
    .unwrap();
}

/// Nhập: bắt buộc nguồn đồng ý, báo mọi lỗi một lần, không đăng ký lại người đã huỷ.
#[tokio::test]
async fn nhap_nguoi_dang_ky() {
    TestDatabase::run(async |test| {
        let h = service(test, config()).await;
        let weekly = list(&h.service, "weekly", true).await;
        let bad = ImportInput {
            people: vec![
                ImportPerson { email: "ok@example.com".to_owned(), ..ImportPerson::default() },
                ImportPerson { email: "nope".to_owned(), ..ImportPerson::default() },
            ],
            list_ids: vec![weekly.row.id, Uuid::now_v7()],
            ..ImportInput::default()
        };
        let error = h.service.import_subscribers(&admin(), bad).await.unwrap_err();
        let mut fields: Vec<&str> =
            error.fields().iter().map(|field| field.field.as_str()).collect();
        fields.sort_unstable();
        assert_eq!(fields, ["consent_source", "list_ids[1]", "subscribers[1].email"]);

        import(
            &h.service,
            weekly.row.id,
            &["ada@example.com".to_owned(), "bob@example.com".to_owned()],
        )
        .await;
        let ada = h
            .service
            .list_subscribers(
                &admin(),
                &crate::subscribers::SubscriberFilter {
                    email: Some("ADA@".to_owned()),
                    ..Default::default()
                },
                &page(),
            )
            .await
            .unwrap()
            .data
            .remove(0);
        assert_eq!(ada.subscriptions[0].consent.as_ref().unwrap()["source"], "test");
        let change = SubscriberChange {
            unsubscribe_list_ids: vec![weekly.row.id],
            ..SubscriberChange::default()
        };
        h.service
            .update_subscriber(&admin(), ada.row.id, change, Some(ada.row.updated_at))
            .await
            .unwrap();
        let input = ImportInput {
            people: vec![
                ImportPerson {
                    email: "ada@example.com".to_owned(),
                    name: Some("Ada".to_owned()),
                    ..ImportPerson::default()
                },
                ImportPerson { email: "bob@example.com".to_owned(), ..ImportPerson::default() },
                ImportPerson {
                    email: "cy@example.com".to_owned(),
                    locale: Some(Locale::Vi),
                    ..ImportPerson::default()
                },
            ],
            list_ids: vec![weekly.row.id],
            consent_source: Some("event-2026".to_owned()),
            ..ImportInput::default()
        };
        let report = h.service.import_subscribers(&admin(), input).await.unwrap();
        assert_eq!(
            (report.created, report.subscribed, report.unchanged, report.kept_unsubscribed),
            (1, 1, 1, 1)
        );
        let ada = h.service.get_subscriber(&admin(), ada.row.id).await.unwrap();
        assert_eq!(
            (ada.row.name.as_deref(), ada.subscriptions[0].row.status.as_str()),
            (Some("Ada"), "unsubscribed")
        );
        let filter = crate::subscribers::SubscriberFilter {
            list_id: Some(weekly.row.id),
            status: Some(crate::subscribers::SubscriptionStatus::Confirmed),
            ..Default::default()
        };
        assert_eq!(
            h.service.list_subscribers(&admin(), &filter, &page()).await.unwrap().data.len(),
            2
        );
    })
    .await
    .unwrap();
}

/// Xuất / xoá theo email; dòng của danh sách chặn được giữ; event không có email.
#[tokio::test]
async fn xuat_va_xoa_theo_email() {
    TestDatabase::run(async |test| {
        let h = service(test, config()).await;
        let weekly = list(&h.service, "weekly", true).await;
        import(&h.service, weekly.row.id, &["ada@example.com".to_owned()]).await;
        let draft = campaign(&h.service, weekly.row.id, false).await;
        super::send(&h.service, draft.row.id).await;
        drain(&h).await;
        h.service.add_suppression(&admin(), "ADA@example.com", None).await.unwrap();

        let data = h.service.email_data(&admin(), "Ada@Example.com").await.unwrap();
        assert_eq!(data.subscriber.as_ref().unwrap().subscriptions.len(), 1);
        assert_eq!(data.deliveries.len(), 1);
        assert_eq!(data.suppression.as_ref().unwrap().reason, "manual");
        assert_eq!(
            h.service.email_data(&user(&["read"]), "ada@example.com").await.unwrap_err().code(),
            "permission_denied"
        );
        assert_eq!(
            h.service.email_data(&admin(), "nope").await.unwrap_err().code(),
            "validation_failed"
        );

        assert!(h.service.erase_email(&admin(), "ada@example.com").await.unwrap());
        assert!(!h.service.erase_email(&admin(), "ada@example.com").await.unwrap());
        let after = h.service.email_data(&admin(), "ada@example.com").await.unwrap();
        assert!(after.subscriber.is_none() && after.deliveries.is_empty());
        assert!(after.suppression.is_some(), "the suppression is kept");
        assert_eq!(count_events(&h.database, "newsletter.subscriber_erased").await, 1);
        for (name, payload) in events(&h.database).await {
            assert!(!payload.contains("ada@"), "{name}: {payload}");
        }
    })
    .await
    .unwrap();
}

/// Dọn dẹp: đăng ký chưa xác nhận quá hạn và người chỉ có nó bị xoá; chạy lại không lỗi.
#[tokio::test]
async fn don_dang_ky_chua_xac_nhan() {
    TestDatabase::run(async |test| {
        let h = service(test, config()).await;
        let weekly = list(&h.service, "weekly", true).await;
        subscribe(&h.service, "ada@example.com", &["weekly"], "203.0.113.9").await.unwrap();
        import(&h.service, weekly.row.id, &["bob@example.com".to_owned()]).await;
        assert_eq!(h.service.cleanup(chrono::Utc::now()).await.unwrap().subscriptions, 0);
        let later = chrono::Utc::now() + chrono::Duration::days(31);
        let cleaned = h.service.cleanup(later).await.unwrap();
        assert_eq!((cleaned.subscriptions, cleaned.subscribers), (1, 1));
        assert_eq!(subscribers(&h.database).await, 1);
        assert_eq!(h.service.cleanup(later).await.unwrap(), crate::retention::Cleaned::default());
    })
    .await
    .unwrap();
}

/// Bộ tra giả cho `content.entry`.
struct Entries;

#[async_trait::async_trait]
impl ResourceResolver for Entries {
    async fn resolve(&self, id: &str) -> Result<Option<Resource>, Error> {
        Ok(Some(Resource {
            public: !id.starts_with("00000000"),
            title: "A new post".to_owned(),
            summary: "What it is about.".to_owned(),
            url: Some("/blog/a-new-post".to_owned()),
            locale: Some("en".to_owned()),
            owner_user_ids: Vec::new(),
        }))
    }
}

fn published(entry: Uuid, kind: &str) -> Delivered {
    Delivered {
        id: Uuid::now_v7(),
        module: "content",
        name: ENTRY_PUBLISHED.to_owned(),
        actor: Actor::system(),
        subject: Some(format!("content.entry:{entry}")),
        payload: json!({"published": {"entry_id": entry, "kind": kind, "locale": "en"}}),
        occurred_at: chrono::Utc::now(),
    }
}

/// Bài mới: event giao hai lần (kể cả song song) chỉ sinh một chiến dịch mỗi danh sách; chế độ
/// `send` gửi luôn; trang và bài không công khai không sinh gì.
#[tokio::test]
async fn bai_moi_mot_chien_dich() {
    TestDatabase::run(async |test| {
        let h = service(test, config()).await;
        h.resources.register("content", "content.entry", Arc::new(Entries)).unwrap();
        let drafts = list(&h.service, "drafts", true).await;
        let change = ListInput { new_posts: Some(NewPosts::Draft), ..ListInput::default() };
        h.service.update_list(&admin(), drafts.row.id, change, None).await.unwrap();
        let sends = list(&h.service, "sends", true).await;
        let change = ListInput { new_posts: Some(NewPosts::Send), ..ListInput::default() };
        h.service.update_list(&admin(), sends.row.id, change, None).await.unwrap();
        let vietnamese = list(&h.service, "vietnamese", true).await;
        let change = ListInput {
            new_posts: Some(NewPosts::Draft),
            new_post_locale: Some(Some(Locale::Vi)),
            ..ListInput::default()
        };
        h.service.update_list(&admin(), vietnamese.row.id, change, None).await.unwrap();
        import(&h.service, sends.row.id, &["ada@example.com".to_owned()]).await;

        let entry = Uuid::now_v7();
        let handler = NewPostHandler(h.service.clone());
        let event = published(entry, "post");
        let (a, b) = tokio::join!(handler.handle(&event), handler.handle(&event));
        a.unwrap();
        b.unwrap();
        handler.handle(&published(entry, "post")).await.unwrap();
        handler.handle(&published(Uuid::now_v7(), "page")).await.unwrap();
        handler.handle(&published(Uuid::nil(), "post")).await.unwrap();
        let all = h
            .service
            .list_campaigns(&admin(), &CampaignFilter::default(), &page())
            .await
            .unwrap()
            .data;
        assert_eq!(all.len(), 2, "one per list (the Vietnamese-only list skips an English post)");
        let draft = all.iter().find(|c| c.row.list_id == drafts.row.id).unwrap();
        assert_eq!(draft.status(), CampaignStatus::Draft);
        assert_eq!(draft.row.subject, "A new post");
        assert!(
            draft.row.body_text.contains("https://www.example.com/blog/a-new-post"),
            "{}",
            draft.row.body_text
        );
        assert_eq!(
            draft.row.source_ref.as_deref(),
            Some(format!("{entry}:{}", drafts.row.id).as_str())
        );
        let sent = all.iter().find(|c| c.row.list_id == sends.row.id).unwrap();
        assert_eq!(sent.status(), CampaignStatus::Sending);
        drain(&h).await;
        assert_eq!(h.mailer.to("ada@example.com").len(), 1);
        assert_eq!(count_events(&h.database, "newsletter.campaign_created").await, 2);
        // Bản nháp gửi tay được.
        h.service
            .change_state(&admin(), draft.row.id, Action::Send, Some(draft.row.updated_at))
            .await
            .unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tai_khoan_bi_xoa_va_du_lieu_ca_nhan() {
    TestDatabase::run(async |test| {
        let h = service(test, config()).await;
        let owner = admin();
        let input = ListInput {
            key: Some("weekly".to_owned()),
            name: Some(Localized { en: Some("Weekly".to_owned()), vi: None }),
            ..ListInput::default()
        };
        let weekly = h.service.create_list(&owner, input).await.unwrap();
        let exported = h.service.export(owner.user_id()).await.unwrap();
        assert_eq!(exported["lists"][0]["key"], "weekly");
        let deleted = Delivered {
            id: Uuid::now_v7(),
            module: "auth",
            name: USER_DELETED.to_owned(),
            actor: Actor::system(),
            subject: None,
            payload: json!({"before": {"id": owner.user_id()}}),
            occurred_at: chrono::Utc::now(),
        };
        h.service.handle(&deleted).await.unwrap();
        h.service.handle(&deleted).await.unwrap();
        assert_eq!(h.service.get_list(&owner, weekly.row.id).await.unwrap().row.created_by, None);
        assert_eq!(h.service.export(owner.user_id()).await.unwrap()["lists"], json!([]));
    })
    .await
    .unwrap();
}
