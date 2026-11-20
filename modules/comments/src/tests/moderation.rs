//! Kiểm duyệt, báo cáo, mail báo trả lời, dữ liệu cá nhân, dọn theo thời hạn, event của module
//! sở hữu đích.

use std::sync::Arc;

use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::http::ClientInfo;
use autoxide_kernel::pagination::Order;
use autoxide_kernel::privacy::{PersonalData, USER_DELETED};
use autoxide_testkit::TestDatabase;
use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, PaginatorTrait, Set};
use serde_json::json;

use super::{
    KIND, POST, as_guest, content_events, counts, events, guest, member, open_config, page, signed,
    visitor,
};
use crate::config::CommentsConfig;
use crate::entity::{comment, profile, subscription};
use crate::moderation::QueueFilter;
use crate::moderation::reports::{ReportReason, all_oldest};
use crate::posting::targets::OwnerEvents;
use crate::service::Status;
use crate::testing::{admin, config, service};
use autoxide_kernel::database::Database;

fn delivered(
    database: &Database,
    name: &str,
    subject: Option<String>,
    payload: serde_json::Value,
) -> Delivered {
    Delivered::new(
        autoxide_kernel::tenancy::TenantContext::default_on(database.clone()),
        "test",
        name.to_owned(),
        payload,
    )
    .subject(subject)
}

/// Nhiều báo cáo tới cùng lúc vượt ngưỡng: bình luận về chờ duyệt ĐÚNG MỘT LẦN (số đếm, event).
/// Mỗi người báo một lần; người kiểm duyệt duyệt lại thì báo cáo sau không ẩn nữa.
#[tokio::test]
async fn bao_cao_dong_thoi_toi_nguong_chi_giu_mot_lan() {
    TestDatabase::run(async |test| {
        let h = service(test, CommentsConfig { report_threshold: 3, ..open_config() }).await;
        let posted =
            as_guest(&h.service, &visitor("203.0.113.5"), guest("Ada", "Ý kiến")).await.unwrap();
        let id = posted.comment.row.id;
        let mut tasks = Vec::new();
        for _ in 0..6 {
            let (service, reporter) = (h.service.clone(), crate::testing::user(&[]));
            tasks.push(tokio::spawn(async move {
                service
                    .report(
                        &service.databases.default_tenant(),
                        Some(&reporter),
                        &ClientInfo::default(),
                        id,
                        ReportReason::Abuse,
                        Some("rude"),
                    )
                    .await
            }));
        }
        // Báo cáo tới SAU khi bình luận đã bị giữ thì không thấy nó nữa (`comments.not_found`), như
        // người lạ — tuỳ thứ tự chạy, có thể vài báo cáo cuối rơi vào trường hợp đó.
        let mut accepted: i32 = 0;
        for task in tasks {
            match task.await.unwrap() {
                Ok(()) => accepted += 1,
                Err(error) => assert_eq!(error.code(), "comments.not_found"),
            }
        }
        assert!(accepted >= 3, "{accepted}");
        let row = h.service.find(&h.tenant, id).await.unwrap();
        assert_eq!(
            (row.status.as_str(), row.hold_reason.as_deref(), row.report_count),
            ("pending", Some("reported"), accepted)
        );
        assert_eq!(counts(&h.database, POST).await, (0, 1));
        let held = events(&h.database)
            .await
            .into_iter()
            .filter(|(name, _)| name == "comments.comment_status_changed")
            .count();
        assert_eq!(held, 1);
        let reports = h.service.reports_of(&h.tenant, &admin(), id, &all_oldest()).await.unwrap();
        assert_eq!(reports.data.len(), usize::try_from(accepted).unwrap());
        assert_eq!(reports.data[0].note.as_deref(), Some("rude"));

        // Bị giữ thì không còn báo cáo được (người lạ không thấy nó).
        let guest_report = h
            .service
            .report(&h.tenant, None, &ClientInfo::default(), id, ReportReason::Spam, None)
            .await;
        assert_eq!(guest_report.unwrap_err().code(), "comments.not_found");
        // Duyệt lại: báo cáo sau không ẩn nữa; báo lại cùng người không thêm dòng.
        h.service.set_status(&h.tenant, &admin(), id, Status::Published, None).await.unwrap();
        for _ in 0..2 {
            h.service
                .report(&h.tenant, None, &ClientInfo::default(), id, ReportReason::Spam, None)
                .await
                .unwrap();
        }
        for _ in 0..3 {
            h.service
                .report(
                    &h.tenant,
                    Some(&crate::testing::user(&[])),
                    &ClientInfo::default(),
                    id,
                    ReportReason::Other,
                    None,
                )
                .await
                .unwrap();
        }
        let row = h.service.find(&h.tenant, id).await.unwrap();
        assert_eq!((row.status.as_str(), row.report_count), ("published", accepted + 4));
        assert_eq!(counts(&h.database, POST).await, (1, 0));
    })
    .await
    .unwrap();
}

/// Duyệt / từ chối / spam / giữ lại với `If-Match`, số đếm theo; xoá hẳn cả luồng; hàng chờ có
/// lọc và tìm không dấu; event không mang chữ, tên hay email.
#[tokio::test]
async fn duyet_xoa_han_hang_cho_va_event() {
    TestDatabase::run(async |test| {
        let h = service(test, config()).await;
        let moderator = admin();
        let client = visitor("203.0.113.6");
        let mut input = guest("Ada Lovelace", "Đường đến Đà Lạt đẹp lắm");
        input.email = Some("ada@example.com".to_owned());
        let posted = as_guest(&h.service, &client, input).await.unwrap();
        let id = posted.comment.row.id;
        assert_eq!(counts(&h.database, POST).await, (0, 1));

        let queue = h
            .service
            .queue(
                &h.tenant,
                &moderator,
                &QueueFilter {
                    search: Some("duong den da lat".to_owned()),
                    ..QueueFilter::default()
                },
                &page(Order::Desc),
            )
            .await
            .unwrap();
        assert_eq!(queue.data.len(), 1);
        assert_eq!(queue.data[0].row.author_email.as_deref(), Some("ada@example.com"));
        let none = h
            .service
            .queue(
                &h.tenant,
                &moderator,
                &QueueFilter { status: Some(Status::Published), ..QueueFilter::default() },
                &page(Order::Desc),
            )
            .await
            .unwrap();
        assert!(none.data.is_empty());
        let reader = crate::testing::user(&["read"]);
        assert_eq!(
            h.service
                .set_status(&h.tenant, &reader, id, Status::Published, None)
                .await
                .unwrap_err()
                .code(),
            "permission_denied"
        );

        let stale = posted.comment.row.updated_at - chrono::Duration::seconds(1);
        assert_eq!(
            h.service
                .set_status(&h.tenant, &moderator, id, Status::Published, Some(stale))
                .await
                .unwrap_err()
                .code(),
            "precondition_failed"
        );
        let approved = h
            .service
            .set_status(
                &h.tenant,
                &moderator,
                id,
                Status::Published,
                Some(posted.comment.row.updated_at),
            )
            .await
            .unwrap();
        assert!(approved.row.published_at.is_some() && approved.row.hold_reason.is_none());
        assert_eq!(counts(&h.database, POST).await, (1, 0));
        h.service.set_status(&h.tenant, &moderator, id, Status::Rejected, None).await.unwrap();
        assert_eq!(counts(&h.database, POST).await, (0, 0));
        h.service.set_status(&h.tenant, &moderator, id, Status::Published, None).await.unwrap();
        assert_eq!(
            h.service
                .set_status(&h.tenant, &moderator, id, Status::Deleted, None)
                .await
                .unwrap_err()
                .fields()[0]
                .field,
            "status"
        );

        // Xoá hẳn bình luận gốc: cả luồng.
        let mut reply = guest("Bea", "Trả lời");
        reply.parent_id = Some(id);
        let reply = as_guest(&h.service, &client, reply).await.unwrap().comment.row.id;
        h.service.set_status(&h.tenant, &moderator, reply, Status::Published, None).await.unwrap();
        assert_eq!(counts(&h.database, POST).await, (2, 0));
        assert_eq!(h.service.hard_delete(&h.tenant, &moderator, id).await.unwrap(), 2);
        assert_eq!(counts(&h.database, POST).await, (0, 0));
        assert_eq!(comment::Entity::find().count(h.database.read()).await.unwrap(), 0);
        assert_eq!(
            h.service.hard_delete(&h.tenant, &moderator, id).await.unwrap_err().code(),
            "comments.not_found"
        );

        let names: Vec<String> = events(&h.database)
            .await
            .into_iter()
            .map(|(name, payload)| {
                for private in ["ada@example.com", "Lovelace", "Đà Lạt", "Trả lời", "Bea"] {
                    assert!(!payload.contains(private), "{name}: {payload}");
                }
                name
            })
            .collect();
        for expected in [
            "comments.comment_created",
            "comments.comment_published",
            "comments.comment_status_changed",
            "comments.comment_deleted",
        ] {
            assert!(names.iter().any(|name| name == expected), "{names:?}");
        }
    })
    .await
    .unwrap();
}

/// Mail báo trả lời: chỉ email đã xác minh, không chứa chữ do người lạ gõ, không gửi cho chính
/// người trả lời, giao lại không gửi hai lần, huỷ theo dõi bằng token trong mail.
#[tokio::test]
async fn mail_bao_tra_loi() {
    TestDatabase::run(async |test| {
        let h = service(test, open_config()).await;
        let client = visitor("203.0.113.10");
        let mut first = guest("Ada", "Câu hỏi của tôi");
        first.email = Some("ada@example.com".to_owned());
        first.notify_replies = true;
        let root = as_guest(&h.service, &client, first).await.unwrap().comment.row.id;
        let verify = h.mailer.sent()[0].text.clone();
        let token: String = verify
            .split("token=")
            .nth(1)
            .unwrap()
            .chars()
            .take_while(|c| !c.is_whitespace())
            .collect();

        // Chưa xác minh: không có mail báo.
        let mut early = guest("Bea", "Trả lời sớm");
        early.parent_id = Some(root);
        let early = as_guest(&h.service, &client, early).await.unwrap().comment.row.id;
        assert_eq!(h.service.notify_reply(&h.tenant, early).await.unwrap(), 0);
        h.service.verify(None, &token).await.unwrap();

        let mut reply = guest("Spammer Joe", "BUY CHEAP PILLS at shady.example");
        reply.parent_id = Some(root);
        let reply = as_guest(&h.service, &client, reply).await.unwrap().comment.row.id;
        let handler = crate::notify::ReplyNotifier(Arc::clone(&h.service));
        let event = delivered(
            &h.database,
            "comments.comment_published",
            None,
            json!({"after": {"id": reply}}),
        );
        handler.handle(&event).await.unwrap();
        handler.handle(&event).await.unwrap();
        let mails: Vec<_> = h
            .mailer
            .sent()
            .into_iter()
            .filter(|mail| mail.subject.starts_with("New reply"))
            .collect();
        assert_eq!(mails.len(), 1, "a redelivered event sends no second mail");
        let mail = &mails[0];
        assert_eq!(mail.to, "ada@example.com");
        assert!(mail.subject.contains("\"Hello world\""), "{}", mail.subject);
        assert!(mail.text.contains("https://www.example.com/posts/public-1"), "{}", mail.text);
        for stranger in ["Spammer", "PILLS", "shady"] {
            assert!(
                !mail.text.contains(stranger) && !mail.subject.contains(stranger),
                "{}",
                mail.text
            );
        }
        // Người theo dõi tự trả lời mình: không mail.
        let mut own = guest("Ada", "Cảm ơn");
        own.parent_id = Some(root);
        own.email = Some("ada@example.com".to_owned());
        let own = as_guest(&h.service, &client, own).await.unwrap().comment.row.id;
        assert_eq!(h.service.notify_reply(&h.tenant, own).await.unwrap(), 0);

        // Huỷ theo dõi bằng token trong mail; token lạ không phải lỗi.
        let unsubscribe: String = mail
            .text
            .split("unsubscribe?token=")
            .nth(1)
            .unwrap()
            .chars()
            .take_while(|c| !c.is_whitespace())
            .collect();
        assert_eq!(h.service.unsubscribe(None, "unknown", false).await.unwrap(), 0);
        assert_eq!(h.service.unsubscribe(None, &unsubscribe, false).await.unwrap(), 1);
        assert_eq!(subscription::Entity::find().count(h.database.read()).await.unwrap(), 0);
        let mut later = guest("Cid", "Muộn");
        later.parent_id = Some(root);
        let later = as_guest(&h.service, &client, later).await.unwrap().comment.row.id;
        assert_eq!(h.service.notify_reply(&h.tenant, later).await.unwrap(), 0);
    })
    .await
    .unwrap();
}

/// Xuất / xoá theo email (khách), tài khoản bị xoá (`USER_DELETED`, nhận hai lần), xuất dữ liệu
/// theo tài khoản; event không mang email.
#[tokio::test]
async fn du_lieu_ca_nhan() {
    TestDatabase::run(async |test| {
        let h = service(test, open_config()).await;
        let moderator = admin();
        let client = visitor("203.0.113.11");
        let mut first = guest("Ada", "Gốc của Ada");
        first.email = Some("ada@example.com".to_owned());
        let root = as_guest(&h.service, &client, first).await.unwrap().comment.row.id;
        let mut second = guest("Ada", "Thêm");
        second.email = Some("ADA@example.com".to_owned());
        as_guest(&h.service, &client, second).await.unwrap();
        let mut reply = guest("Bea", "Trả lời Ada");
        reply.parent_id = Some(root);
        as_guest(&h.service, &client, reply).await.unwrap();

        let reader = crate::testing::user(&["read"]);
        assert_eq!(
            h.service.email_data(&h.tenant, &reader, "ada@example.com").await.unwrap_err().code(),
            "permission_denied"
        );
        assert_eq!(
            h.service.email_data(&h.tenant, &moderator, "nope").await.unwrap_err().fields()[0]
                .field,
            "email"
        );
        let data = h.service.email_data(&h.tenant, &moderator, "Ada@Example.com").await.unwrap();
        assert_eq!(data.comments.len(), 2);
        assert_eq!(
            h.service.erase_email(&h.tenant, &moderator, "ada@example.com").await.unwrap(),
            2
        );
        let cleared = h.service.find(&h.tenant, root).await.unwrap();
        assert_eq!(
            (
                cleared.status.as_str(),
                cleared.body.as_str(),
                cleared.author_email,
                cleared.ip_fingerprint
            ),
            ("deleted", "", None, None)
        );
        assert!(
            h.service
                .email_data(&h.tenant, &moderator, "ada@example.com")
                .await
                .unwrap()
                .comments
                .is_empty()
        );
        assert_eq!(counts(&h.database, POST).await, (1, 0));
        // Luồng vẫn còn chỗ của bình luận gốc (có trả lời).
        assert_eq!(
            h.service
                .thread(&h.tenant, KIND, POST, &page(Order::Desc), 3, None)
                .await
                .unwrap()
                .data
                .len(),
            1
        );

        // Tài khoản bị xoá.
        let bob = member(&h.service, "Bob", &[]).await;
        let mine = h
            .service
            .post(&h.tenant, Some(&bob), &ClientInfo::default(), signed("Của Bob"))
            .await
            .unwrap()
            .comment
            .row
            .id;
        h.service
            .report(
                &h.tenant,
                Some(&bob),
                &ClientInfo::default(),
                mine,
                crate::moderation::reports::ReportReason::Other,
                None,
            )
            .await
            .unwrap();
        let exported = h.service.export(&h.tenant, bob.user_id()).await.unwrap();
        assert_eq!(exported["profile"]["display_name"], "Bob");
        assert_eq!(exported["comments"][0]["body"], "Của Bob");
        let event =
            delivered(&h.database, USER_DELETED, None, json!({"before": {"id": bob.user_id()}}));
        h.service.handle(&event).await.unwrap();
        h.service.handle(&event).await.unwrap();
        let gone = h.service.find(&h.tenant, mine).await.unwrap();
        assert_eq!((gone.status.as_str(), gone.user_id, gone.body.as_str()), ("deleted", None, ""));
        assert!(
            profile::Entity::find_by_id((h.tenant.id().as_uuid(), bob.user_id()))
                .one(h.database.read())
                .await
                .unwrap()
                .is_none()
        );
        let exported = h.service.export(&h.tenant, bob.user_id()).await.unwrap();
        assert_eq!(
            (
                exported["comments"].clone(),
                exported["reports"].clone(),
                exported["profile"].clone()
            ),
            (json!([]), json!([]), json!(null))
        );

        for (name, payload) in events(&h.database).await {
            assert!(
                !payload.contains("example.com") && !payload.contains("Của Bob"),
                "{name}: {payload}"
            );
        }
    })
    .await
    .unwrap();
}

/// Job dọn: chờ duyệt / spam chưa từng hiện quá hạn bị xoá (gốc còn trả lời thì xoá mềm); đã
/// từng hiện thì không; dấu vết IP quá hạn bị xoá, số đếm đúng; chạy lại không đổi gì.
#[tokio::test]
async fn don_theo_thoi_han() {
    TestDatabase::run(async |test| {
        let h = service(test, open_config()).await;
        let moderator = admin();
        let client = visitor("203.0.113.12");
        let root = as_guest(&h.service, &client, guest("Ada", "Gốc")).await.unwrap().comment.row.id;
        let mut reply = guest("Bea", "Trả lời");
        reply.parent_id = Some(root);
        as_guest(&h.service, &client, reply).await.unwrap();
        let lonely =
            as_guest(&h.service, &client, guest("Cid", "Một mình")).await.unwrap().comment.row.id;
        h.service.set_status(&h.tenant, &moderator, root, Status::Pending, None).await.unwrap();
        h.service.set_status(&h.tenant, &moderator, lonely, Status::Spam, None).await.unwrap();
        assert_eq!(counts(&h.database, POST).await, (1, 1));

        let now = crate::service::now();
        let early = h.service.cleanup(&h.service.databases, &h.database, now).await.unwrap();
        assert_eq!((early.deleted, early.cleared, early.fingerprints), (0, 0, 0));
        // Cả hai từng hiện (rồi bị giữ lại / đánh spam): không bao giờ tự bị xoá.
        let shown_once = h
            .service
            .cleanup(&h.service.databases, &h.database, now + chrono::Duration::days(31))
            .await
            .unwrap();
        assert_eq!((shown_once.deleted, shown_once.cleared), (0, 0));
        // Coi như chưa từng hiện (gốc có trả lời mà chưa từng hiện không xảy ra qua API; đặt
        // thẳng để thử nhánh xoá mềm).
        for id in [root, lonely] {
            let mut row = h.service.find(&h.tenant, id).await.unwrap().into_active_model();
            row.published_at = Set(None);
            row.update(h.database.write()).await.unwrap();
        }
        let later = h
            .service
            .cleanup(&h.service.databases, &h.database, now + chrono::Duration::days(31))
            .await
            .unwrap();
        assert_eq!((later.deleted, later.cleared), (1, 1));
        assert!(
            comment::Entity::find_by_id(lonely).one(h.database.read()).await.unwrap().is_none()
        );
        assert_eq!(h.service.find(&h.tenant, root).await.unwrap().status, "deleted");
        assert_eq!(counts(&h.database, POST).await, (1, 0));
        let ips = h
            .service
            .cleanup(&h.service.databases, &h.database, now + chrono::Duration::days(91))
            .await
            .unwrap();
        assert_eq!((ips.deleted, ips.cleared, ips.fingerprints), (0, 0, 1));
        assert_eq!(
            h.service
                .cleanup(&h.service.databases, &h.database, now + chrono::Duration::days(91))
                .await
                .unwrap(),
            crate::retention::Cleaned::default()
        );
    })
    .await
    .unwrap();
}

/// Module sở hữu đích lưu trữ bài: đích đóng; xoá bài: đích đóng và ẩn. Nhận lại cùng event không
/// phát event mới. Quản trị viên mở lại được.
#[tokio::test]
async fn event_cua_chu_dich_dong_va_an() {
    TestDatabase::run(async |test| {
        let h = service(test, open_config()).await;
        let client = visitor("203.0.113.13");
        let id = as_guest(&h.service, &client, guest("Ada", "Trước khi đóng"))
            .await
            .unwrap()
            .comment
            .row
            .id;
        let handler = OwnerEvents {
            service: Arc::clone(&h.service),
            events: crate::posting::targets::owner_events(
                &content_events(),
                &h.service.config.target_types,
            ),
        };
        let subject = Some(format!("{KIND}:{POST}"));
        let archived = delivered(&h.database, "content.entry_archived", subject.clone(), json!({}));
        handler.handle(&archived).await.unwrap();
        handler.handle(&archived).await.unwrap();
        let closed = events(&h.database)
            .await
            .into_iter()
            .filter(|(name, _)| name == "comments.target_closed")
            .count();
        assert_eq!(closed, 1);
        assert_eq!(
            as_guest(&h.service, &client, guest("Ada", "Sau khi đóng")).await.unwrap_err().code(),
            "comments.target_closed"
        );
        assert_eq!(
            h.service
                .thread(&h.tenant, KIND, POST, &page(Order::Desc), 3, None)
                .await
                .unwrap()
                .data
                .len(),
            1,
            "closed, still shown"
        );

        handler
            .handle(&delivered(&h.database, "content.entry_deleted", subject, json!({})))
            .await
            .unwrap();
        assert!(
            h.service
                .thread(&h.tenant, KIND, POST, &page(Order::Desc), 3, None)
                .await
                .unwrap()
                .data
                .is_empty()
        );
        assert_eq!(
            h.service.counts(&h.tenant, KIND, &[POST.to_owned()]).await.unwrap(),
            [(POST.to_owned(), 0, 0)]
        );
        assert_eq!(
            h.service.public_comment(&h.tenant, id).await.unwrap_err().code(),
            "comments.not_found"
        );

        // Mở lại bằng tay; `If-Match` cũ bị từ chối.
        let target = h.service.target(&h.tenant, KIND, POST).await.unwrap();
        let change = || crate::posting::targets::TargetChange {
            state: Some("open".to_owned()),
            hidden: Some(false),
            closes_at: None,
        };
        let stale = target.updated_at.unwrap() - chrono::Duration::seconds(1);
        assert_eq!(
            h.service
                .set_target(&h.tenant, &admin(), KIND, POST, change(), Some(stale))
                .await
                .unwrap_err()
                .code(),
            "precondition_failed"
        );
        let reopened = h
            .service
            .set_target(&h.tenant, &admin(), KIND, POST, change(), target.updated_at)
            .await
            .unwrap();
        assert!(reopened.is_open(crate::service::now()));
        assert_eq!(h.service.public_comment(&h.tenant, id).await.unwrap().row.id, id);
        // Tự đóng sau `closes_at`.
        let past = Some(Some(crate::service::now() - chrono::Duration::minutes(1)));
        let auto = crate::posting::targets::TargetChange { closes_at: past, ..Default::default() };
        h.service.set_target(&h.tenant, &admin(), KIND, POST, auto, None).await.unwrap();
        assert_eq!(
            as_guest(&h.service, &client, guest("Ada", "Quá hạn")).await.unwrap_err().code(),
            "comments.target_closed"
        );
    })
    .await
    .unwrap();
}

/// Ba báo cáo vô danh không xoá được bình luận cũ: bình luận đã hiện bị giữ vì báo cáo thì job dọn
/// không bao giờ tự xoá nó; bình luận chờ duyệt chưa từng hiện thì bị xoá. Khách báo cáo gom theo
/// khối /48 (IPv6). Khoá người báo của khách bỏ dấu vết IP sau `IP_RETENTION_DAYS`.
#[tokio::test]
async fn bao_cao_khong_lam_binh_luan_da_hien_bi_xoa() {
    TestDatabase::run(async |test| {
        let config = CommentsConfig {
            report_threshold: 3,
            guest_mode: crate::config::Mode::Pre,
            user_mode: crate::config::Mode::Post,
            ..config()
        };
        let h = service(test, config).await;
        let bob = member(&h.service, "Bob", &[]).await;
        let shown = h
            .service
            .post(&h.tenant, Some(&bob), &ClientInfo::default(), signed("Ý kiến cũ"))
            .await
            .unwrap();
        assert_eq!(shown.shown, Status::Published);
        let shown = shown.comment.row.id;
        let waiting =
            as_guest(&h.service, &visitor("203.0.113.40"), guest("Ada", "Chưa ai duyệt")).await;
        let waiting = waiting.unwrap().comment.row.id;
        assert_eq!(counts(&h.database, POST).await, (1, 1));

        // Hai địa chỉ trong cùng /48 là một người báo; khối /48 khác, IPv4 là người khác.
        for ip in ["2001:db8:1:1::1", "2001:db8:1:2::9", "2001:db8:2::1"] {
            h.service
                .report(&h.tenant, None, &visitor(ip), shown, ReportReason::Abuse, None)
                .await
                .unwrap();
        }
        assert_eq!(h.service.find(&h.tenant, shown).await.unwrap().report_count, 2);
        let user = crate::testing::user(&[]);
        h.service
            .report(
                &h.tenant,
                Some(&user),
                &ClientInfo::default(),
                shown,
                ReportReason::Abuse,
                None,
            )
            .await
            .unwrap();
        let held = h.service.find(&h.tenant, shown).await.unwrap();
        assert_eq!(
            (held.status.as_str(), held.hold_reason.as_deref()),
            ("pending", Some("reported"))
        );
        assert_eq!(counts(&h.database, POST).await, (0, 2));

        let now = crate::service::now();
        let cleaned = h
            .service
            .cleanup(&h.service.databases, &h.database, now + chrono::Duration::days(31))
            .await
            .unwrap();
        assert_eq!((cleaned.deleted, cleaned.cleared), (1, 0));
        assert!(
            comment::Entity::find_by_id(waiting).one(h.database.read()).await.unwrap().is_none()
        );
        let kept = h.service.find(&h.tenant, shown).await.unwrap();
        assert_eq!((kept.status.as_str(), kept.body.as_str()), ("pending", "Ý kiến cũ"));
        assert_eq!(counts(&h.database, POST).await, (0, 1));
        // Người kiểm duyệt quyết: duyệt lại thì hiện như cũ.
        h.service.set_status(&h.tenant, &admin(), shown, Status::Published, None).await.unwrap();
        assert_eq!(counts(&h.database, POST).await, (1, 0));

        // Quá hạn giữ dấu vết IP: khoá người báo của khách thành `expired:<id>`, của người dùng
        // giữ nguyên; chạy lại không đổi gì.
        let before =
            h.service.reports_of(&h.tenant, &admin(), shown, &all_oldest()).await.unwrap().data;
        assert_eq!(before.iter().filter(|r| r.reporter_key.starts_with("ip:")).count(), 2);
        let later = h
            .service
            .cleanup(&h.service.databases, &h.database, now + chrono::Duration::days(91))
            .await
            .unwrap();
        assert_eq!(later.reporters, 2);
        let after =
            h.service.reports_of(&h.tenant, &admin(), shown, &all_oldest()).await.unwrap().data;
        let keys: Vec<String> = after.iter().map(|r| r.reporter_key.clone()).collect();
        let expected: Vec<String> = before
            .iter()
            .map(|r| {
                if r.reporter_key.starts_with("ip:") {
                    format!("expired:{}", r.id)
                } else {
                    r.reporter_key.clone()
                }
            })
            .collect();
        assert_eq!(keys, expected);
        assert_eq!(keys.iter().filter(|key| key.starts_with("user:")).count(), 1);
        assert_eq!(
            h.service
                .cleanup(&h.service.databases, &h.database, now + chrono::Duration::days(91))
                .await
                .unwrap()
                .reporters,
            0
        );
    })
    .await
    .unwrap();
}

/// Job dọn đọc một lô rồi mới xoá từng dòng: bình luận được duyệt (hay đổi trạng thái) giữa hai
/// bước thì không bị xoá, số đếm không lệch; lần dọn sau xét lại theo trạng thái mới.
#[tokio::test]
async fn duyet_trong_luc_don_thi_khong_xoa() {
    TestDatabase::run(async |test| {
        let h = service(test, config()).await;
        let moderator = admin();
        let client = visitor("203.0.113.41");
        let approved =
            as_guest(&h.service, &client, guest("Ada", "Sẽ được duyệt")).await.unwrap().comment.row;
        let marked = as_guest(&h.service, &client, guest("Bea", "Sẽ bị đánh spam"))
            .await
            .unwrap()
            .comment
            .row;
        assert_eq!(counts(&h.database, POST).await, (0, 2));
        let stale_approved = h.service.find(&h.tenant, approved.id).await.unwrap();
        let stale_marked = h.service.find(&h.tenant, marked.id).await.unwrap();

        h.service
            .set_status(&h.tenant, &moderator, approved.id, Status::Published, None)
            .await
            .unwrap();
        h.service.set_status(&h.tenant, &moderator, marked.id, Status::Spam, None).await.unwrap();
        assert_eq!(h.service.expire(&h.tenant, &stale_approved).await.unwrap(), None);
        assert_eq!(h.service.expire(&h.tenant, &stale_marked).await.unwrap(), None);
        assert_eq!(h.service.find(&h.tenant, approved.id).await.unwrap().status, "published");
        assert_eq!(h.service.find(&h.tenant, marked.id).await.unwrap().status, "spam");
        assert_eq!(counts(&h.database, POST).await, (1, 0));

        // Lần dọn sau (đọc mới): spam chưa từng hiện bị xoá, bình luận đã duyệt thì không.
        let later = crate::service::now() + chrono::Duration::days(31);
        let cleaned = h.service.cleanup(&h.service.databases, &h.database, later).await.unwrap();
        assert_eq!((cleaned.deleted, cleaned.cleared), (1, 0));
        assert!(
            comment::Entity::find_by_id(marked.id).one(h.database.read()).await.unwrap().is_none()
        );
        assert_eq!(h.service.find(&h.tenant, approved.id).await.unwrap().status, "published");
        assert_eq!(counts(&h.database, POST).await, (1, 0));
    })
    .await
    .unwrap();
}

/// Thao tác hàng loạt (bước 51 P8): duyệt bình luận đã duyệt thì bỏ qua (job chạy lại không làm
/// hai lần), spam đổi trạng thái, xoá hẳn xoá; id lạ là `comments.not_found`.
#[tokio::test]
async fn hanh_dong_hang_loat_cua_binh_luan() {
    TestDatabase::run(async |test| {
        let h = service(test, open_config()).await;
        let posted =
            as_guest(&h.service, &visitor("203.0.113.5"), guest("Ada", "Ý kiến")).await.unwrap();
        let id = posted.comment.row.id.to_string();
        let actions = crate::bulk::actions(&h.service);
        let named =
            |name: &str| actions.iter().find(|action| action.info().action == name).unwrap();
        let tenant = &h.tenant;
        let moderator = admin();
        let approve = named("approve").apply(tenant, &moderator, &id).await.unwrap();
        assert_eq!(approve, autoxide_kernel::bulk::BulkOutcome::Skipped("unchanged"));
        let spam = named("spam").apply(tenant, &moderator, &id).await.unwrap();
        assert_eq!(spam, autoxide_kernel::bulk::BulkOutcome::Done);
        let row = h.service.find(tenant, posted.comment.row.id).await.unwrap();
        assert_eq!(row.status, "spam");
        let unknown = named("delete").apply(tenant, &moderator, "x").await.unwrap_err();
        assert_eq!(unknown.code(), "comments.not_found");
        named("delete").apply(tenant, &moderator, &id).await.unwrap();
        assert!(h.service.find(tenant, posted.comment.row.id).await.is_err());
    })
    .await
    .unwrap();
}

/// Vụ việc của `compliance` ẩn bình luận rồi hiện lại đúng trạng thái cũ (bước 37, D-363); nhận lại
/// cùng event không hỏng gì; người kiểm duyệt không đổi được bình luận đang bị ẩn; sổ đối tượng
/// biết tác giả.
#[tokio::test]
async fn vu_viec_an_roi_hien_lai_binh_luan() {
    TestDatabase::run(async |test| {
        let h = service(test, open_config()).await;
        let client = visitor("203.0.113.9");
        let posted =
            as_guest(&h.service, &client, guest("Ada", "Hàng này là hàng giả")).await.unwrap();
        let id = posted.comment.row.id;
        assert_eq!(counts(&h.database, POST).await, (1, 0));
        let resolver = crate::restriction::CommentResolver;
        use autoxide_kernel::resource::ResourceResolver;
        let found = resolver.resolve(&h.tenant, &id.to_string()).await.unwrap().unwrap();
        assert!(found.public);

        let notice = autoxide_kernel::moderation::Restriction::new(
            uuid::Uuid::now_v7(),
            format!("comments.comment:{id}"),
            "other",
        );
        let listener = crate::restriction::Listener(h.service.clone());
        let restrict = delivered(
            &h.database,
            autoxide_kernel::moderation::TARGET_RESTRICTED,
            None,
            serde_json::to_value(&notice).unwrap(),
        );
        listener.handle(&restrict).await.unwrap();
        listener.handle(&restrict).await.unwrap();
        let row = comment::Entity::find_by_id(id).one(h.database.read()).await.unwrap().unwrap();
        assert_eq!(
            (row.status.as_str(), row.restricted_status.as_deref()),
            ("rejected", Some("published"))
        );
        assert_eq!(counts(&h.database, POST).await, (0, 0));
        let error = h
            .service
            .set_status(&h.tenant, &admin(), id, Status::Published, None)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "comments.restricted");

        let restore = delivered(
            &h.database,
            autoxide_kernel::moderation::TARGET_RESTORED,
            None,
            serde_json::to_value(&notice).unwrap(),
        );
        listener.handle(&restore).await.unwrap();
        listener.handle(&restore).await.unwrap();
        let row = comment::Entity::find_by_id(id).one(h.database.read()).await.unwrap().unwrap();
        assert_eq!((row.status.as_str(), row.restricted_status), ("published", None));
        assert_eq!(counts(&h.database, POST).await, (1, 0));
    })
    .await
    .unwrap();
}

/// Quản trị ẩn đích đúng lúc đích bị đổi hạn đóng: lệnh ẩn không bị ghi đè. Postgres từng cho bên
/// đổi hạn đọc `hidden = false` trước khi lệnh ẩn commit rồi ghi lại giá trị cũ.
#[tokio::test]
async fn an_dich_dong_thoi_voi_doi_han_khong_mat_lenh_an() {
    TestDatabase::run(async |test| {
        let h = service(test, open_config()).await;
        let client = visitor("203.0.113.14");
        as_guest(&h.service, &client, guest("Ada", "Mở đích")).await.unwrap();
        let later = Some(Some(crate::service::now() + chrono::Duration::days(30)));
        for _ in 0..4 {
            let hide =
                crate::posting::targets::TargetChange { hidden: Some(true), ..Default::default() };
            let extend =
                crate::posting::targets::TargetChange { closes_at: later, ..Default::default() };
            let admin = admin();
            let (hidden, extended) = tokio::join!(
                h.service.set_target(&h.tenant, &admin, KIND, POST, hide, None),
                h.service.set_target(&h.tenant, &admin, KIND, POST, extend, None),
            );
            hidden.unwrap();
            extended.unwrap();
            let target = h.service.target(&h.tenant, KIND, POST).await.unwrap();
            assert!(target.hidden, "{target:?}");
            let show = crate::posting::targets::TargetChange {
                state: Some("open".to_owned()),
                hidden: Some(false),
                closes_at: None,
            };
            h.service.set_target(&h.tenant, &admin, KIND, POST, show, None).await.unwrap();
        }
    })
    .await
    .unwrap();
}
