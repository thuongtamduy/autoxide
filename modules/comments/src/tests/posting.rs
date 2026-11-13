//! Tạo, sửa, xoá bình luận; pipeline kiểm duyệt; chống spam; đọc luồng.

use std::time::Duration;

use autoxide_kernel::antispam::AntispamConfig;
use autoxide_kernel::error::ErrorKind;
use autoxide_kernel::http::ClientInfo;
use autoxide_kernel::pagination::{Order, PageRequest};
use autoxide_testkit::TestDatabase;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, PaginatorTrait, QueryFilter, Set,
};

use super::{
    KIND, POST, as_guest, counts, events, guest, member, open_config, page, signed, token, visitor,
};
use crate::config::{CommentsConfig, Mode};
use crate::entity::{ban, comment};
use crate::moderation::NewBan;
use crate::posting::NewComment;
use crate::service::Status;
use crate::testing::{admin, config, service, service_with};

/// Khách lần đầu bị giữ (mặc định `pre`); email đã xác minh KHÔNG giúp lần sau vượt duyệt —
/// "đã từng được duyệt" chỉ tính theo tài khoản, vì khách gõ email nào cũng được. Người dùng
/// đăng nhập (`first_time`) bị giữ lần đầu, được duyệt rồi thì lần sau hiện ngay.
#[tokio::test]
async fn khach_lan_dau_bi_giu_email_xac_minh_khong_vuot_duyet_nguoi_dung_thi_co() {
    TestDatabase::run(async |test| {
        let h = service(test, config()).await;
        let moderator = admin();
        let client = visitor("203.0.113.7");

        let mut first = guest("Adelaide", "Bài viết hay quá!");
        first.email = Some(" Ada@Example.com ".to_owned());
        first.notify_replies = true;
        first.locale = Some("vi".to_owned());
        let posted = as_guest(&h.service, &client, first).await.unwrap();
        assert_eq!(posted.shown, Status::Pending);
        assert_eq!(posted.comment.row.hold_reason.as_deref(), Some("moderation"));
        assert!(posted.edit_token.is_some());
        // Mail xác minh: không chứa chữ do khách gõ (tên, bài), chỉ tiêu đề đích và link.
        let mails = h.mailer.sent();
        assert_eq!(mails.len(), 1);
        assert_eq!(mails[0].to, "ada@example.com");
        assert!(mails[0].text.contains("\"Hello world\""), "{}", mails[0].text);
        assert!(!mails[0].text.contains("Adelaide") && !mails[0].text.contains("hay quá"));
        assert!(mails[0].subject.contains("Xác nhận"), "Vietnamese, as the comment");
        let link = mails[0].text.split("token=").nth(1).unwrap();
        let token_value: String = link.chars().take_while(|c| !c.is_whitespace()).collect();
        h.service.verify(&token_value).await.unwrap();
        assert_eq!(
            h.service.verify(&token_value).await.unwrap_err().code(),
            "comments.link_invalid",
            "a token works once"
        );
        let approved = h
            .service
            .set_status(&moderator, posted.comment.row.id, Status::Published, None)
            .await
            .unwrap();
        assert!(approved.row.email_verified);

        // Lần sau, cùng email đã xác minh: vẫn chờ duyệt.
        let mut again = guest("Ada", "Thêm một ý nữa.");
        again.email = Some("ada@example.com".to_owned());
        let second = as_guest(&h.service, &client, again).await.unwrap();
        assert_eq!(second.shown, Status::Pending);
        // Chế độ `first_time` cho khách cũng vậy: khách không có tài khoản để nhớ.
        let h2 = service(test, CommentsConfig { guest_mode: Mode::FirstTime, ..config() }).await;
        let mut third = guest("Ada", "Ý thứ ba.");
        third.email = Some("ada@example.com".to_owned());
        assert_eq!(as_guest(&h2.service, &client, third).await.unwrap().shown, Status::Pending);

        // Người dùng: lần đầu chờ, được duyệt rồi thì lần sau hiện ngay.
        let bob = member(&h.service, "Bob", &[]).await;
        let first = h
            .service
            .post(Some(&bob), &ClientInfo::default(), signed("Chào mọi người"))
            .await
            .unwrap();
        assert_eq!(
            (first.shown, first.comment.row.hold_reason.as_deref()),
            (Status::Pending, Some("first_comment"))
        );
        assert_eq!(first.edit_token, None, "users edit as themselves");
        h.service
            .set_status(&moderator, first.comment.row.id, Status::Published, None)
            .await
            .unwrap();
        let next =
            h.service.post(Some(&bob), &ClientInfo::default(), signed("Lại là tôi")).await.unwrap();
        assert_eq!(next.shown, Status::Published);
        assert_eq!(next.comment.name.as_deref(), Some("Bob"));
        assert_eq!(counts(&h.database, POST).await, (3, 2));
    })
    .await
    .unwrap();
}

/// Bẫy, quá nhiều lần: câu trả lời giống hệt bài bị giữ chờ duyệt, không lưu gì. Quá nhanh: lưu
/// ở `spam`, cùng câu trả lời. Token thiếu / sai / hết hạn: lỗi 400.
#[tokio::test]
async fn chong_spam_cung_mot_cau_tra_loi_token_hong_la_loi() {
    TestDatabase::run(async |test| {
        let h = service(test, CommentsConfig { per_ip_per_hour: 2, ..config() }).await;
        let client = visitor("198.51.100.4");
        let held = as_guest(&h.service, &client, guest("Ada", "Người thật")).await.unwrap();

        let mut trapped = guest("Bot", "mua ngay");
        trapped.trap = Some("http://spam.example".to_owned());
        let trapped = as_guest(&h.service, &client, trapped).await.unwrap();
        assert_eq!(trapped.shown, held.shown);
        assert!(trapped.edit_token.is_some());
        assert_eq!(trapped.comment.row.id.get_version_num(), 7);
        // Xoá thử bằng ID / token giả: trả lời như xoá bài thật đang chờ duyệt.
        let fake_token = trapped.edit_token.as_deref();
        h.service.delete_own(None, trapped.comment.row.id, fake_token).await.unwrap();
        h.service.delete_own(None, held.comment.row.id, held.edit_token.as_deref()).await.unwrap();

        as_guest(&h.service, &client, guest("Ada", "Lần hai")).await.unwrap();
        let limited = as_guest(&h.service, &client, guest("Ada", "Lần ba")).await.unwrap();
        assert_eq!(limited.shown, Status::Pending);
        assert_eq!(comment::Entity::find().count(h.database.read()).await.unwrap(), 2);
        assert_eq!(h.service.find(held.comment.row.id).await.unwrap().status, "deleted");

        // Token thiếu, sai, của phạm vi khác.
        let mut missing = guest("Ada", "Không token");
        assert_eq!(
            h.service.post(None, &client, missing.clone()).await.unwrap_err().code(),
            "antispam.token_missing"
        );
        missing.form_token = Some("garbage".to_owned());
        let error = h.service.post(None, &client, missing.clone()).await.unwrap_err();
        assert_eq!(error.kind(), ErrorKind::BadRequest);
        assert_eq!(error.code(), "antispam.token_invalid");
        missing.form_token = Some(token(&h.service, "media.item").await);
        assert_eq!(
            h.service.post(None, &client, missing).await.unwrap_err().code(),
            "antispam.token_invalid"
        );

        // Quá nhanh: lưu ở `spam`, người gửi thấy như bài chờ duyệt.
        let slow = AntispamConfig::default().min_fill(Duration::from_secs(600));
        let h2 = service_with(test, config(), slow).await;
        let fast =
            as_guest(&h2.service, &visitor("192.0.2.1"), guest("Bot", "Nhanh quá")).await.unwrap();
        assert_eq!(fast.shown, Status::Pending);
        let stored = h2.service.find(fast.comment.row.id).await.unwrap();
        assert_eq!(
            (stored.status.as_str(), stored.hold_reason.as_deref()),
            ("spam", Some("too_fast"))
        );
        // Bài spam không tính vào số chờ duyệt (bài chờ là của khách đầu; bài đầu đã xoá).
        assert_eq!(counts(&h2.database, POST).await, (0, 1));
    })
    .await
    .unwrap();
}

/// Gửi lại cùng token form: cùng một bình luận (tuần tự và đồng thời), khách nhận token sửa mới.
#[tokio::test]
async fn gui_lai_cung_token_ra_cung_binh_luan() {
    TestDatabase::run(async |test| {
        let h = service(test, open_config()).await;
        let client = visitor("203.0.113.8");
        let input = |token: &str| NewComment {
            form_token: Some(token.to_owned()),
            ..guest("Ada", "Một lần thôi")
        };
        let same = token(&h.service, KIND).await;
        let first = h.service.post(None, &client, input(&same)).await.unwrap();
        let again = h.service.post(None, &client, input(&same)).await.unwrap();
        assert_eq!(first.comment.row.id, again.comment.row.id);
        assert_ne!(first.edit_token, again.edit_token);
        // Token sửa cũ thôi dùng được, token mới dùng được.
        let id = first.comment.row.id;
        assert_eq!(
            h.service.edit(None, id, first.edit_token.as_deref(), "Sửa").await.unwrap_err().code(),
            "comments.not_author"
        );
        h.service.edit(None, id, again.edit_token.as_deref(), "Sửa").await.unwrap();

        let shared = token(&h.service, KIND).await;
        let mut tasks = Vec::new();
        for _ in 0..5 {
            let (service, client, input) = (
                h.service.clone(),
                client.clone(),
                NewComment { form_token: Some(shared.clone()), ..guest("Ada", "Đồng thời") },
            );
            tasks.push(tokio::spawn(async move { service.post(None, &client, input).await }));
        }
        let mut ids = Vec::new();
        for task in tasks {
            ids.push(task.await.unwrap().unwrap().comment.row.id);
        }
        ids.dedup();
        assert_eq!(ids.len(), 1, "every answer names the same comment");
        assert_eq!(comment::Entity::find().count(h.database.read()).await.unwrap(), 2);
        assert_eq!(counts(&h.database, POST).await, (2, 0));
    })
    .await
    .unwrap();
}

/// Thứ tự cố định của pipeline và lỗi rõ ràng cho từng bước.
#[tokio::test]
async fn pipeline_kiem_duyet() {
    TestDatabase::run(async |test| {
        let base = CommentsConfig {
            banned_words: vec!["casino".to_owned()],
            suspect_words: vec!["giam gia".to_owned()],
            signed_in_only: vec!["media.item".to_owned()],
            ..open_config()
        };
        let h = service(test, base).await;
        let client = visitor("203.0.113.9");
        let code = |result: Result<crate::posting::Posted, autoxide_kernel::error::Error>| {
            result.unwrap_err().code().to_owned()
        };

        // Loại đích lạ, đích chưa công khai / không có.
        let mut other = guest("Ada", "x");
        other.target_type = "forms.form".to_owned();
        assert_eq!(code(as_guest(&h.service, &client, other).await), "comments.target_not_found");
        let mut draft = guest("Ada", "x");
        draft.target_id = "draft-1".to_owned();
        assert_eq!(code(as_guest(&h.service, &client, draft).await), "comments.target_not_found");
        let mut missing = guest("Ada", "x");
        missing.target_id = "nope".to_owned();
        assert_eq!(code(as_guest(&h.service, &client, missing).await), "comments.target_not_found");
        // Loại không có bộ tra: nhận theo cấu hình. Nhưng loại này chỉ cho người đăng nhập.
        let mut media = guest("Ada", "x");
        media.target_type = "media.item".to_owned();
        assert_eq!(code(as_guest(&h.service, &client, media).await), "comments.guests_not_allowed");

        // Mọi lỗi đầu vào một lần.
        let mut bad = guest("admin", " \n ");
        bad.email = Some("not-an-email".to_owned());
        bad.locale = Some("fr".to_owned());
        let error = as_guest(&h.service, &client, bad).await.unwrap_err();
        let mut fields: Vec<String> =
            error.fields().iter().map(|f| format!("{}:{}", f.field, f.code)).collect();
        fields.sort();
        assert_eq!(
            fields,
            ["author_name:reserved", "body:required", "email:email", "locale:invalid"]
        );
        let mut notify = guest("Ada", "Báo tôi");
        notify.notify_replies = true;
        assert_eq!(
            as_guest(&h.service, &client, notify).await.unwrap_err().fields()[0].field,
            "email"
        );

        // Từ cấm: từ chối. Từ nghi ngờ, quá nhiều link: giữ lại kèm lý do.
        assert_eq!(
            code(as_guest(&h.service, &client, guest("Ada", "Chơi CASINO đi")).await),
            "comments.banned_word"
        );
        let suspect = as_guest(&h.service, &client, guest("Ada", "Giảm giá sốc")).await.unwrap();
        assert_eq!(suspect.shown, Status::Pending);
        assert_eq!(
            h.service.find(suspect.comment.row.id).await.unwrap().hold_reason.as_deref(),
            Some("suspect_word")
        );
        let links = as_guest(
            &h.service,
            &client,
            guest("Ada", "https://a.example https://b.example https://c.example"),
        )
        .await
        .unwrap();
        assert_eq!(
            h.service.find(links.comment.row.id).await.unwrap().hold_reason.as_deref(),
            Some("too_many_links")
        );
        let shown =
            as_guest(&h.service, &client, guest("Ada", "Xem https://a.example nhé")).await.unwrap();
        assert_eq!(shown.shown, Status::Published);

        // Trả lời: phải là bình luận đang hiện của cùng đích; trả lời một trả lời gắn vào gốc.
        let mut orphan = guest("Ada", "Trả lời");
        orphan.parent_id = Some(suspect.comment.row.id);
        assert_eq!(code(as_guest(&h.service, &client, orphan).await), "comments.parent_not_found");
        let mut reply = guest("Bea", "Đồng ý");
        reply.parent_id = Some(shown.comment.row.id);
        let reply = as_guest(&h.service, &client, reply).await.unwrap();
        let mut deeper = guest("Cid", "Tôi cũng vậy");
        deeper.parent_id = Some(reply.comment.row.id);
        let deeper = as_guest(&h.service, &client, deeper).await.unwrap().comment.row;
        assert_eq!(
            (deeper.root_id, deeper.parent_id, deeper.reply_to_id, deeper.depth),
            (shown.comment.row.id, Some(shown.comment.row.id), Some(reply.comment.row.id), 1)
        );
        assert_eq!(h.service.find(shown.comment.row.id).await.unwrap().reply_count, 2);

        // Người dùng: cần `comments.post` và tên hiển thị; khách không được gửi tên của người dùng.
        let nobody = crate::testing::user(&["read"]);
        assert_eq!(
            code(h.service.post(Some(&nobody), &client, signed("x")).await),
            "permission_denied"
        );
        let fresh = crate::testing::user(&["post"]);
        assert_eq!(
            code(h.service.post(Some(&fresh), &client, signed("x")).await),
            "comments.profile_required"
        );
        let mut named = signed("x");
        named.author_name = Some("Me".to_owned());
        let dan = member(&h.service, "Dan", &[]).await;
        assert_eq!(
            h.service.post(Some(&dan), &client, named).await.unwrap_err().fields()[0].field,
            "author_name"
        );
        assert_eq!(
            h.service.set_profile(&fresh, "Dan").await.unwrap_err().code(),
            "comments.name_taken"
        );
        assert_eq!(
            h.service.set_profile(&fresh, "Moderator").await.unwrap_err().fields()[0].code,
            "reserved"
        );
        // Tên viết trọn chữ Kirin trông như tên Latin: vẫn là tên dành riêng, vẫn trùng tên
        // người khác (D-151).
        assert_eq!(
            h.service
                .set_profile(&fresh, "\u{0430}\u{0501}\u{043C}\u{0456}\u{043F}")
                .await
                .unwrap_err()
                .fields()[0]
                .code,
            "reserved"
        );
        member(&h.service, "Coco", &[]).await;
        assert_eq!(
            h.service
                .set_profile(&fresh, "\u{0421}\u{041E}\u{0421}\u{041E}")
                .await
                .unwrap_err()
                .code(),
            "comments.name_taken"
        );
        let mut fake = guest("\u{0430}\u{0501}\u{043C}\u{0456}\u{043F}", "Xin chào");
        fake.email = Some("fake@example.com".to_owned());
        assert_eq!(
            as_guest(&h.service, &client, fake).await.unwrap_err().fields()[0].code,
            "reserved"
        );

        // Đích đóng: từ chối. Tắt bình luận: từ chối.
        h.service
            .set_target(
                &admin(),
                KIND,
                POST,
                crate::targets::TargetChange {
                    state: Some("closed".to_owned()),
                    ..Default::default()
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            code(as_guest(&h.service, &client, guest("Ada", "Muộn rồi")).await),
            "comments.target_closed"
        );
        let off = service(test, CommentsConfig { enabled: false, ..config() }).await;
        assert_eq!(
            code(as_guest(&off.service, &client, guest("Ada", "x")).await),
            "comments.disabled"
        );
        let no_guests = service(test, CommentsConfig { allow_guests: false, ..config() }).await;
        let mut elsewhere = guest("Ada", "x");
        elsewhere.target_id = "public-2".to_owned();
        assert_eq!(
            code(as_guest(&no_guests.service, &client, elsewhere).await),
            "comments.guests_not_allowed"
        );
    })
    .await
    .unwrap();
}

/// Nhiều người trả lời cùng một luồng cùng lúc: số đếm của đích và `reply_count` đúng.
#[tokio::test]
async fn tra_loi_dong_thoi_giu_so_dem_dung() {
    TestDatabase::run(async |test| {
        let h = service(test, open_config()).await;
        let root =
            as_guest(&h.service, &visitor("203.0.113.1"), guest("Ada", "Gốc")).await.unwrap();
        let mut tasks = Vec::new();
        for index in 0..8u8 {
            let service = h.service.clone();
            let principal = member(&h.service, &format!("User {index}"), &[]).await;
            let mut input = signed(&format!("Trả lời số {index}"));
            input.parent_id = Some(root.comment.row.id);
            tasks.push(tokio::spawn(async move {
                service.post(Some(&principal), &ClientInfo::default(), input).await
            }));
        }
        for task in tasks {
            assert_eq!(task.await.unwrap().unwrap().shown, Status::Published);
        }
        assert_eq!(counts(&h.database, POST).await, (9, 0));
        assert_eq!(h.service.find(root.comment.row.id).await.unwrap().reply_count, 8);
    })
    .await
    .unwrap();
}

/// Tác giả sửa trong thời hạn (người dùng, khách bằng token), sửa chạy lại kiểm nội dung, hết
/// hạn thì từ chối; tự xoá là xoá mềm giữ chỗ trong luồng.
#[tokio::test]
async fn sua_trong_thoi_han_va_xoa_mem() {
    TestDatabase::run(async |test| {
        let h = service(
            test,
            CommentsConfig { suspect_words: vec!["giam gia".to_owned()], ..open_config() },
        )
        .await;
        let client = visitor("203.0.113.2");
        let posted = as_guest(&h.service, &client, guest("Ada", "Bản đầu")).await.unwrap();
        let id = posted.comment.row.id;
        let token = posted.edit_token.clone().unwrap();
        let eve = member(&h.service, "Eve", &[]).await;
        assert_eq!(
            h.service.edit(Some(&eve), id, None, "Của tôi").await.unwrap_err().code(),
            "comments.not_author"
        );
        assert_eq!(
            h.service.edit(None, id, Some("wrong"), "x").await.unwrap_err().code(),
            "comments.not_author"
        );
        let edited = h.service.edit(None, id, Some(&token), "Bản hai").await.unwrap();
        assert_eq!((edited.row.body.as_str(), edited.status()), ("Bản hai", Status::Published));
        assert!(edited.row.edited_at.is_some());
        // Sửa thành chữ nghi ngờ: về chờ duyệt, số đếm theo.
        let held = h.service.edit(None, id, Some(&token), "Giảm giá!").await.unwrap();
        assert_eq!(
            (held.status(), held.row.hold_reason.as_deref()),
            (Status::Pending, Some("suspect_word"))
        );
        assert_eq!(counts(&h.database, POST).await, (0, 1));

        // Hết thời hạn sửa (đẩy lúc tạo về quá khứ).
        let mut row = h.service.find(id).await.unwrap().into_active_model();
        row.created_at = Set(crate::service::now() - chrono::Duration::minutes(16));
        row.update(h.database.write()).await.unwrap();
        assert_eq!(
            h.service.edit(None, id, Some(&token), "Muộn").await.unwrap_err().code(),
            "comments.edit_window_passed"
        );

        // Người dùng tự xoá bình luận gốc có trả lời: chỗ trong luồng giữ lại.
        let root =
            h.service.post(Some(&eve), &client, signed("Gốc của Eve")).await.unwrap().comment.row;
        let mut reply = guest("Bea", "Trả lời Eve");
        reply.parent_id = Some(root.id);
        as_guest(&h.service, &client, reply).await.unwrap();
        h.service.delete_own(Some(&eve), root.id, None).await.unwrap();
        h.service.delete_own(Some(&eve), root.id, None).await.unwrap();
        let gone = h.service.find(root.id).await.unwrap();
        assert_eq!(
            (gone.status.as_str(), gone.body.as_str(), gone.reply_count),
            ("deleted", "", 1)
        );
        let thread = h.service.thread(KIND, POST, &page(Order::Desc), 3).await.unwrap();
        let shown: Vec<_> = thread
            .data
            .iter()
            .map(|root| (root.root.row.id, root.root.name.clone(), root.replies.len()))
            .collect();
        assert_eq!(shown, [(root.id, None, 1)]);
        assert_eq!(counts(&h.database, POST).await, (1, 1));
        assert_eq!(
            h.service.edit(Some(&eve), root.id, None, "x").await.unwrap_err().code(),
            "comments.not_editable"
        );
        let deleted: Vec<String> = events(&h.database)
            .await
            .into_iter()
            .filter(|(name, _)| name == "comments.comment_deleted")
            .map(|(_, payload)| payload)
            .collect();
        assert_eq!(deleted.len(), 1);
        assert!(deleted[0].contains("\"hard\":false"), "{}", deleted[0]);
    })
    .await
    .unwrap();
}

/// Lệnh cấm có `until`: đang cấm thì từ chối; hết hạn thì gửi lại được.
#[tokio::test]
async fn lenh_cam_het_han_thi_gui_lai_duoc() {
    TestDatabase::run(async |test| {
        let h = service(test, open_config()).await;
        let moderator = admin();
        let bob = member(&h.service, "Bob", &[]).await;
        let client = ClientInfo::default();
        let until = crate::service::now() + chrono::Duration::hours(1);
        let ban = h
            .service
            .ban(
                &moderator,
                NewBan {
                    kind: "user".to_owned(),
                    value: bob.user_id().to_string(),
                    reason: "Spam".to_owned(),
                    until: Some(until),
                },
            )
            .await
            .unwrap();
        assert_eq!(
            h.service.post(Some(&bob), &client, signed("Xin chào")).await.unwrap_err().code(),
            "comments.banned"
        );
        // Cấm theo email và dấu vết IP.
        h.service
            .ban(
                &moderator,
                NewBan {
                    kind: "email".to_owned(),
                    value: "Spammer@Example.com".to_owned(),
                    reason: "Spam".to_owned(),
                    until: None,
                },
            )
            .await
            .unwrap();
        let mut by_email = guest("Sam", "Hi");
        by_email.email = Some("spammer@example.com".to_owned());
        assert_eq!(
            as_guest(&h.service, &visitor("192.0.2.9"), by_email).await.unwrap_err().code(),
            "comments.banned"
        );
        let fingerprint = autoxide_kernel::ip_hash::fingerprint(
            &h.service.secret,
            crate::service::MODULE,
            "192.0.2.77".parse().unwrap(),
        )
        .await
        .unwrap();
        h.service
            .ban(
                &moderator,
                NewBan {
                    kind: "ip".to_owned(),
                    value: fingerprint,
                    reason: "Flood".to_owned(),
                    until: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            as_guest(&h.service, &visitor("192.0.2.78"), guest("Sam", "Hi"))
                .await
                .unwrap_err()
                .code(),
            "comments.banned",
            "same /24 block"
        );
        // Dữ liệu sai báo một lần.
        let error = h
            .service
            .ban(
                &moderator,
                NewBan {
                    kind: "phone".to_owned(),
                    value: String::new(),
                    reason: String::new(),
                    until: Some(crate::service::now()),
                },
            )
            .await
            .unwrap_err();
        assert_eq!(error.fields().len(), 3);

        // Hết hạn.
        let mut row = ban::Entity::find_by_id(ban.id)
            .one(h.database.read())
            .await
            .unwrap()
            .unwrap()
            .into_active_model();
        row.until = Set(Some(crate::service::now() - chrono::Duration::seconds(1)));
        row.update(h.database.write()).await.unwrap();
        assert_eq!(
            h.service.post(Some(&bob), &client, signed("Xin chào")).await.unwrap().shown,
            Status::Published
        );
        let active = h
            .service
            .bans(
                &moderator,
                &crate::moderation::BanFilter { kind: None, active: true },
                &page(Order::Desc),
            )
            .await
            .unwrap();
        assert_eq!(active.data.len(), 2);
        h.service.unban(&moderator, ban.id).await.unwrap();
        assert_eq!(
            h.service.unban(&moderator, ban.id).await.unwrap_err().code(),
            "comments.ban_not_found"
        );
        // Event không mang giá trị bị cấm.
        for (name, payload) in events(&h.database).await {
            if name.starts_with("comments.ban_") {
                assert!(
                    !payload.contains("example.com")
                        && !payload.contains(&bob.user_id().to_string()),
                    "{payload}"
                );
            }
        }
    })
    .await
    .unwrap();
}

/// Cursor của luồng: mới / cũ trước, không trùng không sót; trả lời kèm theo và trang trả lời.
#[tokio::test]
async fn cursor_cua_luong() {
    TestDatabase::run(async |test| {
        let h = service(test, open_config()).await;
        let writer = member(&h.service, "Writer", &[]).await;
        let client = ClientInfo::default();
        let mut roots = Vec::new();
        for index in 0..5 {
            roots.push(
                h.service
                    .post(Some(&writer), &client, signed(&format!("Gốc {index}")))
                    .await
                    .unwrap()
                    .comment
                    .row
                    .id,
            );
        }
        for index in 0..4 {
            let mut reply = signed(&format!("Trả lời {index}"));
            reply.parent_id = Some(roots[0]);
            h.service.post(Some(&writer), &client, reply).await.unwrap();
        }
        let mut seen = Vec::new();
        let mut request = PageRequest::first(2, Order::Desc);
        loop {
            let found = h.service.thread(KIND, POST, &request, 2).await.unwrap();
            seen.extend(found.data.iter().map(|root| root.root.row.id));
            for root in &found.data {
                if root.root.row.id == roots[0] {
                    assert_eq!(root.replies.len(), 2);
                    assert_eq!(root.root.row.reply_count, 4);
                    assert_eq!(root.replies[0].row.body, "Trả lời 0", "replies oldest first");
                }
            }
            match found.pagination.next_cursor.as_deref().and_then(|cursor| cursor.parse().ok()) {
                Some(cursor) => request = PageRequest::first(2, Order::Desc).after(cursor),
                None => break,
            }
        }
        let mut newest_first = roots.clone();
        newest_first.reverse();
        assert_eq!(seen, newest_first);
        let oldest = h.service.thread(KIND, POST, &page(Order::Asc), 0).await.unwrap();
        assert_eq!(oldest.data.iter().map(|root| root.root.row.id).collect::<Vec<_>>(), roots);
        assert!(oldest.data[0].replies.is_empty());
        let replies =
            h.service.replies(roots[0], &PageRequest::first(3, Order::Asc)).await.unwrap();
        assert_eq!(replies.data.len(), 3);
        assert!(replies.pagination.next_cursor.is_some());
        // Bình luận chờ duyệt không có trong luồng công khai.
        let held = service(test, config()).await;
        as_guest(&held.service, &visitor("203.0.113.3"), guest("Ada", "Chờ")).await.unwrap();
        assert_eq!(
            held.service.thread(KIND, POST, &page(Order::Desc), 3).await.unwrap().data.len(),
            5
        );
    })
    .await
    .unwrap();
}

/// Sửa bình luận đã được duyệt: chế độ `pre` (khách) giữ lại bản sửa, số đếm theo trong cùng
/// transaction; người dùng `first_time` còn bình luận đang hiện thì sửa vẫn hiện. "Đã từng được
/// duyệt" chỉ tính bình luận ĐANG hiện: bình luận được duyệt rồi bị đánh spam không còn tính.
#[tokio::test]
async fn sua_sau_khi_duyet_va_tin_tuong_chi_theo_binh_luan_dang_hien() {
    TestDatabase::run(async |test| {
        let h = service(test, config()).await;
        let moderator = admin();
        let client = visitor("203.0.113.30");

        let posted = as_guest(&h.service, &client, guest("Ada", "Bản sạch")).await.unwrap();
        let id = posted.comment.row.id;
        let token = posted.edit_token.clone().unwrap();
        h.service.set_status(&moderator, id, Status::Published, None).await.unwrap();
        assert_eq!(counts(&h.database, POST).await, (1, 0));
        let edited = h.service.edit(None, id, Some(&token), "Bản bẩn mà không có từ cấm").await;
        let edited = edited.unwrap();
        assert_eq!(
            (edited.status(), edited.row.hold_reason.as_deref()),
            (Status::Pending, Some("moderation"))
        );
        assert!(edited.row.published_at.is_some());
        assert_eq!(counts(&h.database, POST).await, (0, 1));
        // Bình luận đang chờ duyệt mà sửa thì vẫn chờ.
        let again = h.service.edit(None, id, Some(&token), "Bản ba").await.unwrap();
        assert_eq!(again.status(), Status::Pending);
        assert_eq!(counts(&h.database, POST).await, (0, 1));

        // Người dùng `first_time`: lần đầu chờ, được duyệt, sửa vẫn hiện.
        let bob = member(&h.service, "Bob", &[]).await;
        let first = h.service.post(Some(&bob), &client, signed("Lần đầu")).await.unwrap();
        assert_eq!(first.shown, Status::Pending);
        let first = first.comment.row.id;
        h.service.set_status(&moderator, first, Status::Published, None).await.unwrap();
        let edited = h.service.edit(Some(&bob), first, None, "Lần đầu, sửa").await.unwrap();
        assert_eq!(edited.status(), Status::Published);
        assert_eq!(
            h.service.post(Some(&bob), &client, signed("Lần hai")).await.unwrap().shown,
            Status::Published
        );

        // Cara được duyệt một lần, rồi bình luận đó bị đánh spam: lần sau lại chờ duyệt.
        let cara = member(&h.service, "Cara", &[]).await;
        let only = h.service.post(Some(&cara), &client, signed("Một")).await.unwrap();
        let only = only.comment.row.id;
        h.service.set_status(&moderator, only, Status::Published, None).await.unwrap();
        h.service.set_status(&moderator, only, Status::Spam, None).await.unwrap();
        let next = h.service.post(Some(&cara), &client, signed("Hai")).await.unwrap();
        assert_eq!(next.shown, Status::Pending);
        assert_eq!(
            h.service.find(next.comment.row.id).await.unwrap().hold_reason.as_deref(),
            Some("first_comment")
        );
    })
    .await
    .unwrap();
}

/// Trả lời gửi cùng lúc với lệnh xoá hẳn cả luồng: không trả lời nào sống sót mồ côi, trả lời
/// thua thì nhận `comments.parent_not_found`, số đếm của đích khớp với dòng còn lại.
#[tokio::test]
async fn tra_loi_dong_thoi_voi_xoa_han_luong() {
    TestDatabase::run(async |test| {
        let h = service(test, open_config()).await;
        let moderator = admin();
        let mut members = Vec::new();
        for index in 0..12u8 {
            members.push(member(&h.service, &format!("Replier {index}"), &[]).await);
        }
        for (round, group) in members.chunks(4).enumerate() {
            let root = as_guest(
                &h.service,
                &visitor("203.0.113.31"),
                guest("Ada", &format!("Gốc {round}")),
            )
            .await
            .unwrap();
            assert_eq!(root.shown, Status::Published);
            let root = root.comment.row.id;
            let mut tasks = Vec::new();
            for principal in group {
                let (service, principal) = (h.service.clone(), principal.clone());
                let mut input = signed(&format!("Trả lời {round} của {}", principal.user_id()));
                input.parent_id = Some(root);
                tasks.push(tokio::spawn(async move {
                    service.post(Some(&principal), &ClientInfo::default(), input).await.map(|_| ())
                }));
            }
            let (service, moderator) = (h.service.clone(), moderator.clone());
            let deleting = tokio::spawn(async move { service.hard_delete(&moderator, root).await });
            for task in tasks {
                if let Err(error) = task.await.unwrap() {
                    assert_eq!(error.code(), "comments.parent_not_found");
                }
            }
            deleting.await.unwrap().unwrap();
            let left = comment::Entity::find()
                .filter(comment::Column::RootId.eq(root))
                .count(h.database.read())
                .await
                .unwrap();
            assert_eq!(left, 0, "round {round}");
            assert_eq!(counts(&h.database, POST).await, (0, 0), "round {round}");
        }
    })
    .await
    .unwrap();
}
