//! Hai khách trên cùng database (bước 33): cùng đích, cùng tên hiển thị không đụng nhau; mỗi khách
//! chỉ thấy bình luận của mình; token trong link mail của khách kia mang khách của nó.

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::mail::Mailer;
use autoxide_kernel::pagination::Order;
use autoxide_testkit::{TestApp, TestDatabase};

use super::{KIND, POST, guest, open_config, page, signed, visitor};
use crate::posting::{NewComment, Posted};
use crate::service::Status;
use crate::testing::{Harness, admin, service_in, user};

/// Gửi như khách ở khách của `h`, với token form mới của khách đó.
async fn post_as_guest(h: &Harness, ip: &str, mut input: NewComment) -> Result<Posted, Error> {
    let scope = format!("comments:{}", input.target_type);
    input.form_token = Some(h.service.guard.issue(&h.tenant, &scope).await?.token);
    h.service.post(&h.tenant, None, &visitor(ip), input).await
}

/// Token trong link cuối cùng gửi tới `email`.
fn last_token(h: &Harness, email: &str) -> String {
    let mail = h.mailer.sent().into_iter().rev().find(|mail| mail.to == email).unwrap();
    let start = mail.text.find("token=").unwrap() + "token=".len();
    mail.text[start..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect()
}

/// Phần thô của token (bỏ tiền tố khách nếu có).
fn raw(token: &str) -> String {
    token.rsplit('.').next().unwrap().to_owned()
}

#[tokio::test]
async fn hai_khach_cung_dich_cung_ten_khong_dung_nhau_token_mang_khach() {
    TestDatabase::run(async |test| {
        let mailer: Arc<dyn Mailer> = Arc::new(crate::testing::FakeMailer::default());
        let app = TestApp::builder(test)
            .tenant("other")
            .provide::<dyn Mailer>(mailer)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let h = service_in(test, open_config(), &app).await;
        let other = app.tenant("other").await.unwrap();
        let other_id = other.id().as_uuid().simple().to_string();
        let o = Harness { tenant: other, ..h.clone() };

        // Cùng người dùng, cùng tên hiển thị ở hai khách: hai hồ sơ, không `name_taken`.
        let bob = user(&["post"]);
        for harness in [&h, &o] {
            harness.service.set_profile(&harness.tenant, &bob, "Bob").await.unwrap();
        }
        // Cùng đích ở hai khách: mỗi khách một dòng đích, một bình luận.
        let client = visitor("203.0.113.1");
        let posted = h.service.post(&h.tenant, Some(&bob), &client, signed("Của A")).await;
        let mine = posted.unwrap().comment.row;
        let posted = o.service.post(&o.tenant, Some(&bob), &client, signed("Của B")).await;
        let theirs = posted.unwrap().comment.row;
        assert_eq!((mine.status.as_str(), theirs.status.as_str()), ("published", "published"));
        for (harness, row) in [(&h, &mine), (&o, &theirs)] {
            let thread = harness
                .service
                .thread(&harness.tenant, KIND, POST, &page(Order::Desc), 0, None)
                .await
                .unwrap();
            assert_eq!(thread.data.len(), 1);
            assert_eq!(thread.data[0].root.row.id, row.id);
            assert_eq!(thread.data[0].root.name.as_deref(), Some("Bob"));
            let ids = [POST.to_owned()];
            let counts = harness.service.counts(&harness.tenant, KIND, &ids).await.unwrap();
            assert_eq!(counts, vec![(POST.to_owned(), 1, 1)]);
        }
        // Bình luận của khách này không có ở khách kia (đọc, duyệt, xoá).
        assert_eq!(
            o.service.public_comment(&o.tenant, mine.id).await.unwrap_err().code(),
            "comments.not_found"
        );
        let moderator = admin();
        let refused = o.service.set_status(&o.tenant, &moderator, mine.id, Status::Spam, None);
        assert_eq!(refused.await.unwrap_err().code(), "comments.not_found");
        assert_eq!(
            o.service.hard_delete(&o.tenant, &moderator, mine.id).await.unwrap_err().code(),
            "comments.not_found"
        );
        assert_eq!(h.service.find(&h.tenant, mine.id).await.unwrap().status, "published");

        // Mail xác minh của khách kia mang khách của nó; của khách mặc định giữ dạng cũ.
        let ada = "ada@example.com";
        let ask = |body: &str| NewComment {
            email: Some(ada.to_owned()),
            notify_replies: true,
            ..guest("Ada", body)
        };
        let my_question = post_as_guest(&h, "198.51.100.1", ask("Hỏi A")).await.unwrap();
        let my_token = last_token(&h, ada);
        let their_question = post_as_guest(&o, "198.51.100.2", ask("Hỏi B")).await.unwrap();
        let their_token = last_token(&o, ada);
        assert!(!my_token.contains('.'), "default-tenant tokens keep their old form");
        assert!(their_token.starts_with(&format!("t{other_id}.")), "{their_token}");

        // Token của khách mặc định gọi trong khách kia, ghép ID khách kia với phần thô của khách
        // này, hay ID khách không có: token sai, và không bị tiêu mất.
        let invalid = |result: Result<(), Error>| result.unwrap_err().code().to_owned();
        assert_eq!(
            invalid(o.service.verify(Some(&o.tenant), &my_token).await),
            "comments.link_invalid"
        );
        let forged = format!("t{other_id}.{my_token}");
        assert_eq!(invalid(h.service.verify(None, &forged).await), "comments.link_invalid");
        let unknown = format!("t{}.{}", uuid::Uuid::now_v7().simple(), raw(&their_token));
        assert_eq!(invalid(h.service.verify(None, &unknown).await), "comments.link_invalid");
        // Tên miền chính (không biết khách của request): mỗi token xác minh đúng khách của nó.
        h.service.verify(None, &my_token).await.unwrap();
        h.service.verify(None, &their_token).await.unwrap();
        for (harness, question) in [(&h, &my_question), (&o, &their_question)] {
            let found = harness.service.find(&harness.tenant, question.comment.row.id).await;
            assert!(found.unwrap().email_verified);
        }

        // Trả lời ở khách kia: mail báo có token huỷ mang khách; gọi ở tên miền chính huỷ đúng
        // dòng theo dõi ở khách kia.
        let reply = NewComment {
            parent_id: Some(their_question.comment.row.id),
            ..signed("Trả lời B")
        };
        let replied = o.service.post(&o.tenant, Some(&bob), &client, reply).await;
        let replied = replied.unwrap().comment.row;
        assert_eq!(o.service.notify_reply(&o.tenant, replied.id).await.unwrap(), 1);
        let stop = last_token(&o, ada);
        assert!(stop.starts_with(&format!("t{other_id}.")), "{stop}");
        let tampered = format!("t{}.{}", uuid::Uuid::now_v7().simple(), raw(&stop));
        assert_eq!(h.service.unsubscribe(None, &tampered, false).await.unwrap(), 0);
        assert_eq!(h.service.unsubscribe(None, &stop, false).await.unwrap(), 1);
        // Dọn theo thời hạn chạy một lần cho database, theo từng khách.
        let later = crate::service::now() + chrono::Duration::days(400);
        h.service.cleanup(app.registry(), &h.database, later).await.unwrap();
    })
    .await
    .unwrap();
}

/// Cấu hình theo khách (đối chiếu thiết kế 2, A-b7): khách đặt từ cấm, số link, chế độ duyệt thì
/// thay biến môi trường cho riêng khách đó; khách chưa đặt dùng biến môi trường; giá trị sai bị
/// từ chối lúc đặt; bỏ cấu hình thì về giá trị chung.
#[tokio::test]
async fn cau_hinh_module_theo_khach_ghi_de_bien_moi_truong() {
    TestDatabase::run(async |test| {
        let mailer: Arc<dyn Mailer> = Arc::new(crate::testing::FakeMailer::default());
        let app = TestApp::builder(test)
            .tenant("other")
            .user("platform", 1, &["kernel.tenants.manage"])
            .provide::<dyn Mailer>(mailer)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let h = service_in(test, open_config(), &app).await;
        let o = Harness { tenant: app.tenant("other").await.unwrap(), ..h.clone() };
        let base = "/api/v1/kernel/tenants/other/settings";
        for (name, value) in [
            ("comments.banned_words", "casino"),
            ("comments.max_links", "0"),
            ("comments.user_moderation", "pre"),
        ] {
            let response = app
                .put(&format!("{base}/{name}"))
                .token("platform")
                .json(serde_json::json!({ "value": value }))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 200, "{}", response.text());
        }
        // Giá trị sai bị từ chối lúc đặt, lỗi ở ô `value`.
        let refused = app
            .put(&format!("{base}/comments.max_links"))
            .token("platform")
            .json(serde_json::json!({ "value": "500" }))
            .send()
            .await
            .unwrap();
        assert_eq!(refused.status(), 422);
        assert_eq!(refused.json()["errors"][0]["field"], "value");

        let ada = user(&["post"]);
        for harness in [&h, &o] {
            harness.service.set_profile(&harness.tenant, &ada, "Ada").await.unwrap();
        }
        let client = visitor("203.0.113.9");
        // Khách mặc định: biến môi trường (không từ cấm, 2 link, hiện ngay).
        let mine =
            h.service.post(&h.tenant, Some(&ada), &client, signed("casino https://a.example"));
        assert_eq!(mine.await.unwrap().shown, Status::Published);
        // Khách kia: từ cấm của khách.
        let theirs = o.service.post(&o.tenant, Some(&ada), &client, signed("Chơi casino"));
        assert_eq!(theirs.await.unwrap_err().code(), "comments.banned_word");
        // Khách kia: một link đã quá (0) — giữ lại vì link, không phải vì chế độ duyệt.
        let linked = o.service.post(&o.tenant, Some(&ada), &client, signed("https://a.example"));
        let linked = linked.await.unwrap();
        assert_eq!(linked.shown, Status::Pending);
        let stored = o.service.find(&o.tenant, linked.comment.row.id).await.unwrap();
        assert_eq!(stored.hold_reason.as_deref(), Some("too_many_links"));
        // Khách kia: chế độ `pre` cho người dùng đăng nhập.
        let plain = o.service.post(&o.tenant, Some(&ada), &client, signed("Bài hay"));
        let plain = plain.await.unwrap();
        let stored = o.service.find(&o.tenant, plain.comment.row.id).await.unwrap();
        assert_eq!(stored.hold_reason.as_deref(), Some("moderation"));

        // Cài đặt công khai cho website theo khách của request.
        let settings = |slug: &'static str| {
            app.get("/api/v1/comments/public/settings").header("autoxide-tenant", slug).send()
        };
        assert_eq!(settings("other").await.unwrap().json()["data"]["max_links"], 0);
        assert_eq!(settings("default").await.unwrap().json()["data"]["max_links"], 2);

        // Bỏ cấu hình: về biến môi trường.
        let removed =
            app.delete(&format!("{base}/comments.banned_words")).token("platform").send().await;
        assert_eq!(removed.unwrap().status(), 204);
        let again = o.service.post(&o.tenant, Some(&ada), &client, signed("Chơi casino"));
        assert!(again.await.is_ok());
    })
    .await
    .unwrap();
}
