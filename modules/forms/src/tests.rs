//! Test của service `forms` (chạy trên SQLite và Postgres qua `TestDatabase::run`): vòng đời
//! form và phiên bản, kiểm câu trả lời, chống spam hai pha (bẫy / quá nhanh / quá nhiều cùng một
//! câu trả lời, gửi lại cùng token tuần tự và đồng thời), form đóng và `Origin`, event không mang
//! câu trả lời, mail báo, CSV, xuất / xoá theo email, dọn theo thời hạn, tài khoản bị xoá.

use std::net::IpAddr;
use std::time::Duration;

use autoxide_kernel::access::Principal;
use autoxide_kernel::antispam::AntispamConfig;
use autoxide_kernel::error::{Error, ErrorKind};
use autoxide_kernel::event::{Actor, Delivered, EventHandler};
use autoxide_kernel::http::ClientInfo;
use autoxide_kernel::pagination::{Order, PageRequest};
use autoxide_kernel::privacy::{PersonalData, USER_DELETED};
use autoxide_testkit::TestDatabase;
use sea_orm::{ConnectionTrait, EntityTrait, PaginatorTrait, Statement};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::config::FormsConfig;
use crate::entity::submission;
use crate::fields::{Field, Localized};
use crate::forms::{Form, FormInput, FormStatus, scope};
use crate::inbox::{InboxFilter, SubmissionStatus};
use crate::public::{PublicSubmission, Receipt};
use crate::service::FormsService;
use crate::testing::{admin, service, service_with, user};

fn fields(value: Value) -> Vec<Field> {
    serde_json::from_value(value).unwrap()
}

fn contact_fields() -> Vec<Field> {
    fields(json!([
        {"key": "name", "type": "text", "label": {"en": "Name", "vi": "Tên"}, "required": true},
        {"key": "email", "type": "email", "label": {"en": "Email"}, "required": true},
        {"key": "message", "type": "textarea", "label": {"en": "Message", "vi": "Lời nhắn"}},
        {"key": "agree", "type": "consent", "label": {"en": "You may contact me.", "vi": "Bạn được liên hệ với tôi."}}
    ]))
}

fn contact(key: &str) -> FormInput {
    FormInput {
        key: Some(key.to_owned()),
        name: Some("Contact us".to_owned()),
        fields: Some(contact_fields()),
        notify_emails: Some(vec!["Owner@Example.com".to_owned()]),
        success_message: Some(Localized {
            en: Some("Thanks!".to_owned()),
            vi: Some("Cảm ơn!".to_owned()),
        }),
        ..FormInput::default()
    }
}

fn visitor(ip: &str) -> ClientInfo {
    ClientInfo { ip: Some(ip.parse::<IpAddr>().unwrap()), ..ClientInfo::default() }
}

async fn token(service: &FormsService, key: &str) -> String {
    service.guard.issue(&scope(key)).await.unwrap().token
}

fn answers(name: &str, email: &str) -> serde_json::Map<String, Value> {
    json!({"name": name, "email": email, "message": "Hello there", "agree": true})
        .as_object()
        .unwrap()
        .clone()
}

/// Gửi một bài như người lạ, với token mới.
async fn send(
    service: &FormsService,
    key: &str,
    client: &ClientInfo,
    data: serde_json::Map<String, Value>,
) -> Result<Receipt, Error> {
    let input = PublicSubmission {
        data,
        locale: Some("vi".to_owned()),
        page_url: Some("https://example.com/contact".to_owned()),
        form_token: Some(token(service, key).await),
        ..PublicSubmission::default()
    };
    service.submit(key, None, client, input).await
}

async fn count(database: &autoxide_kernel::database::Database) -> u64 {
    submission::Entity::find().count(database.read()).await.unwrap()
}

/// (tên, payload) của mọi event đã ghi, theo thứ tự.
async fn events(database: &autoxide_kernel::database::Database) -> Vec<(String, String)> {
    let backend = database.read().get_database_backend();
    let rows = database
        .read()
        .query_all_raw(Statement::from_string(
            backend,
            "SELECT name, payload FROM forms_events ORDER BY id",
        ))
        .await
        .unwrap();
    rows.into_iter()
        .map(|row| {
            (
                row.try_get::<String>("", "name").unwrap(),
                row.try_get::<String>("", "payload").unwrap(),
            )
        })
        .collect()
}

fn page() -> PageRequest {
    PageRequest::first(50, Order::Desc)
}

#[tokio::test]
async fn vong_doi_form_phien_ban_va_bai_cu_doc_dung_nhan_cu() {
    TestDatabase::run(async |test| {
        let (service, _, database) = service(test, FormsConfig::default()).await;
        let admin = admin();
        let form = service.create_form(&admin, contact("contact")).await.unwrap();
        assert_eq!((form.row.version, form.status()), (1, FormStatus::Open));
        assert_eq!(form.notify_emails, ["owner@example.com"]);

        // Thiếu quyền; key trùng; định nghĩa sai báo mọi lỗi một lần.
        let reader = user(&["read"]);
        assert_eq!(
            service.create_form(&reader, contact("other")).await.unwrap_err().code(),
            "permission_denied"
        );
        assert_eq!(
            service.create_form(&admin, contact("contact")).await.unwrap_err().code(),
            "forms.key_taken"
        );
        let mut bad = contact("Bad Key");
        bad.name = Some(" ".to_owned());
        bad.allowed_origins = Some(vec!["example.com".to_owned()]);
        bad.reply_to_field = Some(Some("name".to_owned()));
        bad.fields = Some(fields(json!([{"key": "x", "type": "select", "label": {}}])));
        let error = service.create_form(&admin, bad).await.unwrap_err();
        let mut paths: Vec<&str> = error.fields().iter().map(|f| f.field.as_str()).collect();
        paths.sort_unstable();
        assert_eq!(
            paths,
            [
                "allowed_origins[0]",
                "fields[0].label",
                "fields[0].options",
                "key",
                "name",
                "reply_to_field"
            ]
        );

        // Một bài gửi theo phiên bản 1.
        let client = visitor("203.0.113.9");
        let first =
            send(&service, "contact", &client, answers("Ada", "ada@example.com")).await.unwrap();

        // Đổi trường: phiên bản 2; sai phiên bản thì 412.
        let mut changed = contact_fields();
        changed[0].label = Localized { en: Some("Full name".to_owned()), vi: None };
        changed.remove(2);
        let input = FormInput { fields: Some(changed), ..FormInput::default() };
        let stale = Some(form.row.updated_at - chrono::Duration::seconds(1));
        assert_eq!(
            service
                .update_form(&admin, form.row.id, input.clone(), stale)
                .await
                .unwrap_err()
                .code(),
            "precondition_failed"
        );
        let updated = service
            .update_form(&admin, form.row.id, input, Some(form.row.updated_at))
            .await
            .unwrap();
        assert_eq!(updated.row.version, 2);
        // Đổi thứ khác trường: không tăng phiên bản.
        let renamed = FormInput { name: Some("Liên hệ".to_owned()), ..FormInput::default() };
        assert_eq!(
            service.update_form(&admin, form.row.id, renamed, None).await.unwrap().row.version,
            2
        );

        // Bài cũ vẫn đọc với nhãn và trường của phiên bản 1.
        let old = service.get_submission(&admin, first.id).await.unwrap();
        assert_eq!((old.row.form_version, old.fields.len()), (1, 4));
        assert_eq!(old.fields[0].label.en.as_deref(), Some("Name"));
        assert_eq!(old.data["message"], "Hello there");
        // Bài mới theo phiên bản 2: trường `message` không còn nhận.
        let mut later = answers("Bob", "bob@example.com");
        assert_eq!(
            send(&service, "contact", &client, later.clone()).await.unwrap_err().fields()[0].field,
            "data.message"
        );
        later.remove("message");
        let second = send(&service, "contact", &client, later).await.unwrap();
        assert_eq!(
            service.get_submission(&admin, second.id).await.unwrap().fields[0].label.en.as_deref(),
            Some("Full name")
        );

        let listed = service.list_forms(&reader, None, &page()).await.unwrap();
        assert_eq!(listed.data.len(), 1);
        // Xoá form: mất cả bài gửi.
        let current = service.get_form(&admin, form.row.id).await.unwrap();
        service.delete_form(&admin, form.row.id, Some(current.row.updated_at)).await.unwrap();
        assert_eq!(count(&database).await, 0);
        assert_eq!(
            service.get_form(&admin, form.row.id).await.unwrap_err().code(),
            "forms.not_found"
        );
        let names: Vec<String> =
            events(&database).await.into_iter().map(|(name, _)| name).collect();
        assert_eq!(names.first().map(String::as_str), Some("forms.form_created"));
        assert_eq!(names.last().map(String::as_str), Some("forms.form_deleted"));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn moi_kieu_truong_duoc_kiem_va_bao_du_loi_mot_lan() {
    TestDatabase::run(async |test| {
        let (service, _, database) = service(test, FormsConfig::default()).await;
        let mut input = contact("survey");
        input.fields = Some(fields(json!([
            {"key": "name", "type": "text", "label": {"en": "Name"}, "required": true, "max_length": 10},
            {"key": "email", "type": "email", "label": {"en": "Email"}},
            {"key": "phone", "type": "phone", "label": {"en": "Phone"}},
            {"key": "note", "type": "textarea", "label": {"en": "Note"}, "min_length": 3},
            {"key": "guests", "type": "number", "label": {"en": "Guests"}, "min": 1, "max": 9},
            {"key": "topic", "type": "select", "label": {"en": "Topic"}, "options": [{"value": "a"}]},
            {"key": "size", "type": "radio", "label": {"en": "Size"}, "options": [{"value": "s"}]},
            {"key": "extras", "type": "checkbox", "label": {"en": "Extras"}, "options": [{"value": "x"}, {"value": "y"}]},
            {"key": "day", "type": "date", "label": {"en": "Day"}},
            {"key": "source", "type": "hidden", "label": {"en": "Source"}},
            {"key": "agree", "type": "consent", "label": {"en": "I agree"}, "required": true}
        ])));
        service.create_form(&admin(), input).await.unwrap();
        let client = visitor("203.0.113.9");
        let wrong = json!({
            "name": "much too long", "email": "nope", "phone": "abc", "note": "no",
            "guests": 0, "topic": "b", "size": ["s"], "extras": ["z"], "day": "12/31/2026",
            "source": 7, "agree": "yes", "hacker": "?"
        });
        let error = send(&service, "survey", &client, wrong.as_object().unwrap().clone()).await.unwrap_err();
        assert_eq!(error.code(), "validation_failed");
        let mut found: Vec<(String, String)> = error.fields().iter().map(|f| (f.field.clone(), f.code.clone())).collect();
        found.sort();
        let expected = [
            ("data.agree", "type"), ("data.day", "date"), ("data.email", "email"), ("data.extras", "option"),
            ("data.guests", "range"), ("data.hacker", "unknown"), ("data.name", "length"), ("data.note", "length"),
            ("data.phone", "phone"), ("data.size", "type"), ("data.source", "type"), ("data.topic", "option"),
        ];
        assert_eq!(found, expected.map(|(a, b)| (a.to_owned(), b.to_owned())));
        assert_eq!(count(&database).await, 0, "nothing stored");

        let right = json!({
            "name": "Ada", "email": "ADA@example.com", "phone": "+84 912 345 678", "note": "Hi!\r\nBye",
            "guests": 2, "topic": "a", "size": "s", "extras": ["y", "x"], "day": "2026-12-31",
            "source": "ads", "agree": true
        });
        let receipt = send(&service, "survey", &client, right.as_object().unwrap().clone()).await.unwrap();
        assert_eq!(receipt.message, "Cảm ơn!");
        let stored = service.get_submission(&admin(), receipt.id).await.unwrap();
        assert_eq!(stored.row.email.as_deref(), Some("ada@example.com"));
        assert_eq!((stored.data["note"].as_str(), stored.data["guests"].as_i64()), (Some("Hi!\nBye"), Some(2)));
        assert_eq!(stored.row.locale.as_deref(), Some("vi"));
        assert!(stored.row.ip_fingerprint.as_deref().is_some_and(|hash| !hash.contains("203.0.113")));
        let consent = stored.consent.unwrap();
        assert_eq!((consent["text"].as_str(), consent["version"].as_str(), consent["source"].as_str()), (Some("I agree"), Some("1"), Some("forms:survey")));
    })
    .await
    .unwrap();
}

/// Bẫy, quá nhanh, quá nhiều lần: người gửi nhận đúng câu trả lời như bài được nhận.
#[tokio::test]
async fn bay_qua_nhanh_qua_nhieu_tra_loi_giong_bai_nhan() {
    TestDatabase::run(async |test| {
        let config = FormsConfig { per_ip_per_hour: 2, ..FormsConfig::default() };
        let (service, _, database) = service(test, config.clone()).await;
        service.create_form(&admin(), contact("contact")).await.unwrap();
        let client = visitor("203.0.113.9");
        let accepted =
            send(&service, "contact", &client, answers("Ada", "ada@example.com")).await.unwrap();
        assert_eq!(count(&database).await, 1);

        // Bẫy: không lưu gì, cùng câu trả lời.
        let trapped = PublicSubmission {
            data: answers("Bot", "bot@example.com"),
            locale: Some("vi".to_owned()),
            form_token: Some(token(&service, "contact").await),
            trap: Some("http://spam.example".to_owned()),
            ..PublicSubmission::default()
        };
        let trapped = service.submit("contact", None, &client, trapped).await.unwrap();
        assert_eq!(trapped.message, accepted.message);
        assert_ne!(trapped.id, accepted.id);
        assert_eq!(trapped.id.get_version_num(), 7, "a fake id looks real");
        assert_eq!(count(&database).await, 1);

        // Quá nhiều lần từ một khối IP (giới hạn 2/giờ): lần thứ ba bị bỏ, cùng câu trả lời.
        let second =
            send(&service, "contact", &client, answers("Bob", "bob@example.com")).await.unwrap();
        let third =
            send(&service, "contact", &visitor("203.0.113.77"), answers("Cy", "cy@example.com"))
                .await
                .unwrap();
        assert_eq!((second.message.as_str(), third.message.as_str()), ("Cảm ơn!", "Cảm ơn!"));
        assert_eq!(count(&database).await, 2);

        // Quá nhanh (token vừa phát chưa đủ 60 giây): lưu ở `spam`, cùng câu trả lời, không event.
        let slow =
            AntispamConfig { min_fill: Duration::from_secs(60), ..AntispamConfig::default() };
        let (strict, mailer, database) = service_with(test, config, slow).await;
        let fast =
            send(&strict, "contact", &visitor("198.51.100.1"), answers("Dee", "dee@example.com"))
                .await
                .unwrap();
        assert_eq!(fast.message, accepted.message);
        let held = strict.get_submission(&admin(), fast.id).await.unwrap();
        assert_eq!(
            (held.status(), held.row.spam_reason.as_deref()),
            (SubmissionStatus::Spam, Some("too_fast"))
        );
        let received = events(&database)
            .await
            .into_iter()
            .filter(|(name, _)| name == "forms.submission_received")
            .count();
        assert_eq!(received, 2, "the spam submission raised no event");
        // Hộp thư mặc định không có spam; lọc `spam` thì thấy.
        let form_id = held.row.form_id;
        let inbox = strict
            .list_submissions(&admin(), form_id, &InboxFilter::default(), &page())
            .await
            .unwrap();
        assert_eq!(inbox.data.len(), 2);
        let spam = InboxFilter { status: Some(SubmissionStatus::Spam), search: None };
        assert_eq!(
            strict.list_submissions(&admin(), form_id, &spam, &page()).await.unwrap().data.len(),
            1
        );
        assert_eq!(strict.notify(fast.id).await.unwrap(), 0, "no mail for spam");
        assert!(mailer.sent().is_empty());

        // Token thiếu, sai: lỗi rõ (người thật tải lại form là xong).
        let missing =
            PublicSubmission { data: answers("E", "e@example.com"), ..PublicSubmission::default() };
        assert_eq!(
            service.submit("contact", None, &client, missing).await.unwrap_err().code(),
            "antispam.token_missing"
        );
    })
    .await
    .unwrap();
}

/// Gửi lại cùng token, lần lượt rồi đồng thời: đúng một bản ghi, mọi lần cùng một ID.
#[tokio::test]
async fn gui_lai_cung_token_chi_mot_ban_ghi() {
    TestDatabase::run(async |test| {
        // Mỗi lần gửi lại vẫn đi qua bộ đếm theo email: nới ra để chỉ thử nonce.
        let config = FormsConfig { per_email_per_hour: 100, ..FormsConfig::default() };
        let (service, _, database) = service(test, config).await;
        service.create_form(&admin(), contact("contact")).await.unwrap();
        let client = visitor("203.0.113.9");
        let input = |token: &str| PublicSubmission {
            data: answers("Ada", "ada@example.com"),
            form_token: Some(token.to_owned()),
            ..PublicSubmission::default()
        };
        let same = token(&service, "contact").await;
        let first = service.submit("contact", None, &client, input(&same)).await.unwrap();
        let again = service.submit("contact", None, &client, input(&same)).await.unwrap();
        assert_eq!(first.id, again.id);
        assert_eq!(count(&database).await, 1);

        let shared = token(&service, "contact").await;
        let mut tasks = Vec::new();
        for _ in 0..5 {
            let (service, client, input) = (service.clone(), client.clone(), input(&shared));
            tasks.push(tokio::spawn(async move {
                service.submit("contact", None, &client, input).await
            }));
        }
        let mut ids = Vec::new();
        for task in tasks {
            ids.push(task.await.unwrap().unwrap().id);
        }
        ids.dedup();
        assert_eq!(ids.len(), 1, "every answer names the same submission");
        assert_eq!(count(&database).await, 2);
        assert!(
            submission::Entity::find_by_id(ids[0]).one(database.read()).await.unwrap().is_some()
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn form_dong_origin_va_nguoi_goi_dang_nhap() {
    TestDatabase::run(async |test| {
        let config = FormsConfig { per_user_per_hour: 1, ..FormsConfig::default() };
        let (service, _, _) = service(test, config).await;
        let mut input = contact("contact");
        input.allowed_origins = Some(vec!["https://Example.com/".to_owned()]);
        let form = service.create_form(&admin(), input).await.unwrap();
        assert_eq!(form.allowed_origins, ["https://example.com"]);

        let mut client = visitor("203.0.113.9");
        client.origin = Some("https://evil.example".to_owned());
        let refused =
            send(&service, "contact", &client, answers("A", "a@example.com")).await.unwrap_err();
        assert_eq!(
            (refused.kind(), refused.code()),
            (ErrorKind::Forbidden, "forms.origin_not_allowed")
        );
        client.origin = Some("https://example.com".to_owned());
        send(&service, "contact", &client, answers("A", "a@example.com")).await.unwrap();
        // Không có `Origin` (máy chủ gọi): không kiểm.
        send(&service, "contact", &visitor("203.0.113.9"), answers("B", "b@example.com"))
            .await
            .unwrap();

        // Đăng nhập mà không có `forms.submit` (tài khoản tự đăng ký): đi đường của người lạ —
        // thiếu token thì bị từ chối như người lạ; có token thì được nhận.
        let member = Principal::new(Uuid::now_v7(), Vec::<String>::new());
        let signed = || PublicSubmission {
            data: answers("C", "c@example.com"),
            ..PublicSubmission::default()
        };
        let refused = service.submit("contact", Some(&member), &client, signed()).await;
        assert_eq!(refused.unwrap_err().code(), "antispam.token_missing");
        let mut with_token = signed();
        with_token.form_token = Some(token(&service, "contact").await);
        service.submit("contact", Some(&member), &client, with_token).await.unwrap();

        // Website gọi bằng API key có `forms.submit`: không cần token; vượt giới hạn theo người
        // dùng thì 429.
        let website = user(&["submit"]);
        service.submit("contact", Some(&website), &client, signed()).await.unwrap();
        let limited =
            service.submit("contact", Some(&website), &client, signed()).await.unwrap_err();
        assert_eq!(limited.code(), "antispam.rate_limited");

        let closed = FormInput { status: Some(FormStatus::Closed), ..FormInput::default() };
        service.update_form(&admin(), form.row.id, closed, None).await.unwrap();
        let error =
            send(&service, "contact", &client, answers("D", "d@example.com")).await.unwrap_err();
        assert_eq!((error.kind(), error.code()), (ErrorKind::Conflict, "forms.closed"));
        assert_eq!(
            send(&service, "nope", &client, answers("D", "d@example.com"))
                .await
                .unwrap_err()
                .code(),
            "forms.not_found"
        );
    })
    .await
    .unwrap();
}

/// Event không mang câu trả lời hay email; mail báo có `Reply-To` và khoá chống trùng, event
/// giao lại không sinh khoá mới; ra khỏi spam thì báo như bài mới.
#[tokio::test]
async fn event_khong_mang_cau_tra_loi_va_mail_bao() {
    TestDatabase::run(async |test| {
        let (service, mailer, database) = service(test, FormsConfig::default()).await;
        let form = service.create_form(&admin(), contact("contact")).await.unwrap();
        let receipt = send(
            &service,
            "contact",
            &visitor("203.0.113.9"),
            answers("Ada Lovelace", "ada@example.com"),
        )
        .await
        .unwrap();
        for (name, payload) in events(&database).await {
            for secret in ["Ada Lovelace", "ada@example.com", "Hello there", "owner@example.com"] {
                assert!(!payload.contains(secret), "{name} carries `{secret}`: {payload}");
            }
        }
        let (_, payload) = events(&database)
            .await
            .into_iter()
            .find(|(name, _)| name == "forms.submission_received")
            .unwrap();
        let payload: Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(payload["after"]["id"], json!(receipt.id));
        assert_eq!(payload["after"]["form_key"], "contact");

        // Event giao hai lần: hai lời gọi `send` cùng `dedupe_key` (module `mail` bỏ lần sau).
        let delivered = Delivered {
            id: Uuid::now_v7(),
            module: "forms",
            name: "forms.submission_received".to_owned(),
            actor: Actor::public(),
            subject: None,
            payload,
            occurred_at: chrono::Utc::now(),
        };
        let notifier = crate::notify::Notifier(service.clone());
        notifier.handle(&delivered).await.unwrap();
        notifier.handle(&delivered).await.unwrap();
        let sent = mailer.sent();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0].dedupe_key, sent[1].dedupe_key);
        assert_eq!(
            (sent[0].to.as_str(), sent[0].reply_to.as_deref()),
            ("owner@example.com", Some("ada@example.com"))
        );
        assert_eq!(sent[0].subject, "New submission: Contact us");
        assert!(
            sent[0].text.contains("Name: Ada Lovelace")
                && sent[0].text.contains("Message: Hello there"),
            "{}",
            sent[0].text
        );

        // Spam → không phải spam: thêm `forms.submission_received`.
        let current = service.get_submission(&admin(), receipt.id).await.unwrap();
        let reviewer = user(&["review"]);
        let spam = service
            .set_submission_status(
                &reviewer,
                receipt.id,
                SubmissionStatus::Spam,
                Some(current.row.updated_at),
            )
            .await
            .unwrap();
        assert_eq!(
            service
                .set_submission_status(
                    &reviewer,
                    receipt.id,
                    SubmissionStatus::New,
                    Some(current.row.updated_at)
                )
                .await
                .unwrap_err()
                .code(),
            "precondition_failed"
        );
        service
            .set_submission_status(
                &reviewer,
                receipt.id,
                SubmissionStatus::New,
                Some(spam.row.updated_at),
            )
            .await
            .unwrap();
        let names: Vec<String> =
            events(&database).await.into_iter().map(|(name, _)| name).collect();
        assert_eq!(names.iter().filter(|name| *name == "forms.submission_received").count(), 2);
        assert_eq!(
            names.iter().filter(|name| *name == "forms.submission_status_changed").count(),
            2
        );
        assert_eq!(
            service
                .set_submission_status(&user(&["read"]), receipt.id, SubmissionStatus::Read, None)
                .await
                .unwrap_err()
                .code(),
            "permission_denied"
        );

        service.delete_submission(&reviewer, receipt.id).await.unwrap();
        assert_eq!(service.notify(receipt.id).await.unwrap(), 0, "deleted since: no mail");
        assert_eq!(
            service.get_submission(&admin(), receipt.id).await.unwrap_err().code(),
            "forms.submission_not_found"
        );
        let _ = form;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn xuat_csv_chan_injection_va_tim_theo_chu() {
    TestDatabase::run(async |test| {
        let (service, _, _) = service(test, FormsConfig::default()).await;
        let form = service.create_form(&admin(), contact("contact")).await.unwrap();
        let mut evil = answers("=HYPERLINK(\"http://evil\",\"x\")", "eve@example.com");
        evil.insert("message".to_owned(), json!("+1, \"quoted\"\n@second line"));
        send(&service, "contact", &visitor("203.0.113.9"), evil).await.unwrap();
        send(
            &service,
            "contact",
            &visitor("203.0.113.9"),
            answers("Đặng Thu Hà", "ha@example.com"),
        )
        .await
        .unwrap();

        assert_eq!(
            service.export_csv(&user(&["read"]), form.row.id, None).await.unwrap_err().code(),
            "permission_denied"
        );
        let (name, text) = service.export_csv(&user(&["export"]), form.row.id, None).await.unwrap();
        assert_eq!(name, "contact-submissions.csv");
        let lines: Vec<&str> = text.split("\r\n").collect();
        assert!(lines[0].starts_with("\u{feff}\"id\",\"created_at\",\"status\""), "{}", lines[0]);
        assert!(lines[0].ends_with("\"name\",\"email\",\"message\",\"agree\""), "{}", lines[0]);
        assert!(text.contains("\"'=HYPERLINK(\"\"http://evil\"\",\"\"x\"\")\""), "{text}");
        assert!(text.contains("\"'+1, \"\"quoted\"\"\n@second line\""), "{text}");
        assert!(text.contains("Đặng Thu Hà"));

        let search = InboxFilter { status: None, search: Some("dang thu".to_owned()) };
        let found =
            service.list_submissions(&admin(), form.row.id, &search, &page()).await.unwrap();
        assert_eq!(found.data.len(), 1);
        assert_eq!(found.data[0].data["name"], "Đặng Thu Hà");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn xuat_va_xoa_theo_email() {
    TestDatabase::run(async |test| {
        let (service, _, database) = service(test, FormsConfig::default()).await;
        service.create_form(&admin(), contact("contact")).await.unwrap();
        service.create_form(&admin(), contact("support")).await.unwrap();
        let client = visitor("203.0.113.9");
        send(&service, "contact", &client, answers("Ada", "Ada@Example.com")).await.unwrap();
        send(&service, "support", &client, answers("Ada", "ada@example.com")).await.unwrap();
        send(&service, "support", &client, answers("Bob", "bob@example.com")).await.unwrap();

        let exporter = user(&["export"]);
        assert_eq!(
            service.submissions_of_email(&admin(), "nope").await.unwrap_err().code(),
            "validation_failed"
        );
        assert_eq!(
            service
                .submissions_of_email(&user(&["read"]), "ada@example.com")
                .await
                .unwrap_err()
                .code(),
            "permission_denied"
        );
        let found = service.submissions_of_email(&exporter, " ADA@example.com ").await.unwrap();
        let keys: Vec<&str> = found.iter().map(|s| s.form_key.as_str()).collect();
        assert_eq!(keys, ["contact", "support"]);

        assert_eq!(service.erase_email(&exporter, "ada@example.com").await.unwrap(), 2);
        assert_eq!(service.erase_email(&exporter, "ada@example.com").await.unwrap(), 0);
        assert_eq!(count(&database).await, 1);
        let (_, payload) = events(&database)
            .await
            .into_iter()
            .find(|(name, _)| name == "forms.submissions_erased")
            .unwrap();
        assert!(!payload.contains("ada@"), "{payload}");
        assert!(payload.contains("\"count\":2"), "{payload}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn don_theo_thoi_han_chay_lai_khong_loi() {
    TestDatabase::run(async |test| {
        let config = FormsConfig {
            retention_days: 0,
            spam_retention_days: 30,
            ip_retention_days: 90,
            ..FormsConfig::default()
        };
        let (service, _, database) = service(test, config).await;
        let kept = service.create_form(&admin(), contact("kept")).await.unwrap();
        let mut short = contact("short");
        short.retention_days = Some(Some(60));
        service.create_form(&admin(), short).await.unwrap();
        let client = visitor("203.0.113.9");
        let held = send(&service, "kept", &client, answers("A", "a@example.com")).await.unwrap();
        send(&service, "kept", &client, answers("B", "b@example.com")).await.unwrap();
        send(&service, "short", &client, answers("C", "c@example.com")).await.unwrap();
        let current = service.get_submission(&admin(), held.id).await.unwrap();
        service
            .set_submission_status(
                &admin(),
                held.id,
                SubmissionStatus::Spam,
                Some(current.row.updated_at),
            )
            .await
            .unwrap();

        let now = chrono::Utc::now();
        assert_eq!(service.cleanup(now).await.unwrap(), crate::retention::Cleaned::default());
        let later = service.cleanup(now + chrono::Duration::days(31)).await.unwrap();
        assert_eq!((later.spam, later.expired, later.fingerprints), (1, 0, 0));
        let much_later = service.cleanup(now + chrono::Duration::days(91)).await.unwrap();
        assert_eq!((much_later.spam, much_later.expired, much_later.fingerprints), (0, 1, 1));
        assert_eq!(
            service.cleanup(now + chrono::Duration::days(91)).await.unwrap(),
            crate::retention::Cleaned::default()
        );
        let left = service
            .list_submissions(&admin(), kept.row.id, &InboxFilter::default(), &page())
            .await
            .unwrap();
        assert_eq!(left.data.len(), 1, "kept forever by default");
        assert_eq!(left.data[0].row.ip_fingerprint, None);
        assert_eq!(count(&database).await, 1);
        let expired = events(&database)
            .await
            .into_iter()
            .filter(|(name, _)| name == "forms.submissions_expired")
            .count();
        assert_eq!(expired, 2);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tai_khoan_bi_xoa_va_du_lieu_ca_nhan() {
    TestDatabase::run(async |test| {
        let (service, _, _) = service(test, FormsConfig::default()).await;
        let owner = admin();
        let form: Form = service.create_form(&owner, contact("contact")).await.unwrap();
        let exported = service.export(owner.user_id()).await.unwrap();
        assert_eq!(exported["forms"][0]["key"], "contact");

        let deleted = Delivered {
            id: Uuid::now_v7(),
            module: "auth",
            name: USER_DELETED.to_owned(),
            actor: Actor::system(),
            subject: None,
            payload: json!({"before": {"id": owner.user_id()}}),
            occurred_at: chrono::Utc::now(),
        };
        service.handle(&deleted).await.unwrap();
        service.handle(&deleted).await.unwrap();
        assert_eq!(service.get_form(&owner, form.row.id).await.unwrap().row.created_by, None);
        assert_eq!(service.export(owner.user_id()).await.unwrap()["forms"], json!([]));
    })
    .await
    .unwrap();
}

/// `forms.manage` thôi thì không đặt / đổi được `notify_emails` (mail báo chứa câu trả lời):
/// cần thêm `forms.read`. Gửi lại đúng danh sách cũ thì không cần.
#[tokio::test]
async fn doi_noi_nhan_mail_bao_can_quyen_doc() {
    TestDatabase::run(async |test| {
        let (service, _, _) = service(test, FormsConfig::default()).await;
        let manager = user(&["manage"]);
        assert_eq!(
            service.create_form(&manager, contact("contact")).await.unwrap_err().code(),
            "permission_denied"
        );
        let mut quiet = contact("contact");
        quiet.notify_emails = None;
        let form = service.create_form(&manager, quiet).await.unwrap();
        assert!(form.notify_emails.is_empty());

        let redirect = || FormInput {
            notify_emails: Some(vec!["thief@example.com".to_owned()]),
            ..FormInput::default()
        };
        let refused = service.update_form(&manager, form.row.id, redirect(), None).await;
        assert_eq!(refused.unwrap_err().code(), "permission_denied");
        let rename = FormInput {
            name: Some("Contact".to_owned()),
            notify_emails: Some(Vec::new()),
            ..FormInput::default()
        };
        service.update_form(&manager, form.row.id, rename, None).await.unwrap();

        let both = user(&["manage", "read"]);
        let changed = service.update_form(&both, form.row.id, redirect(), None).await.unwrap();
        assert_eq!(changed.notify_emails, ["thief@example.com"]);
        let same = FormInput {
            notify_emails: Some(vec!["Thief@Example.com".to_owned()]),
            ..FormInput::default()
        };
        service.update_form(&manager, form.row.id, same, None).await.unwrap();
        let removed = FormInput { notify_emails: Some(Vec::new()), ..FormInput::default() };
        let refused = service.update_form(&manager, form.row.id, removed, None).await;
        assert_eq!(refused.unwrap_err().code(), "permission_denied");
    })
    .await
    .unwrap();
}

/// Bên gửi mail không nhận địa chỉ người gửi làm `Reply-To`: mail báo vẫn đi, không có
/// `Reply-To`, event không lỗi (không bị thử lại tới khi hỏng hẳn).
#[tokio::test]
async fn mail_bao_bo_reply_to_khi_ben_gui_tu_choi() {
    TestDatabase::run(async |test| {
        let (service, mailer, _) = service(test, FormsConfig::default()).await;
        service.create_form(&admin(), contact("contact")).await.unwrap();
        let receipt =
            send(&service, "contact", &visitor("203.0.113.9"), answers("Ada", "ada@example.com"))
                .await
                .unwrap();
        mailer.refuse_reply_to.store(true, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(service.notify(receipt.id).await.unwrap(), 1);
        let sent = mailer.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!((sent[0].to.as_str(), sent[0].reply_to.as_deref()), ("owner@example.com", None));
    })
    .await
    .unwrap();
}

/// Email dài ra khi viết thường (`İ` → `i̇`) bị kiểm theo dạng sẽ lưu: báo lỗi 422, không vỡ cột
/// `email` (`VARCHAR(320)` trên Postgres) thành 500. Dạng vừa giới hạn thì lưu được trên cả hai.
#[tokio::test]
async fn email_dai_ra_khi_viet_thuong_bi_kiem_theo_dang_luu() {
    TestDatabase::run(async |test| {
        let (service, _, database) = service(test, FormsConfig::default()).await;
        service.create_form(&admin(), contact("contact")).await.unwrap();
        let client = visitor("203.0.113.9");
        let long = format!("{}@{}.example.com", "İ".repeat(30), "x".repeat(60));
        let error = send(&service, "contact", &client, answers("A", &long)).await.unwrap_err();
        assert_eq!(error.code(), "validation_failed");
        assert_eq!(error.fields()[0].field, "data.email");

        let fits = format!("{}@{}.example.com", "İ".repeat(20), "x".repeat(60));
        let receipt = send(&service, "contact", &client, answers("B", &fits)).await.unwrap();
        let stored = service.get_submission(&admin(), receipt.id).await.unwrap();
        assert_eq!(stored.row.email, Some(fits.to_lowercase()));
        assert_eq!(count(&database).await, 1);
    })
    .await
    .unwrap();
}

/// Xoá / xuất theo email gặp cả bài mà địa chỉ nằm ở một câu trả lời khác (trường email thứ
/// hai), không chỉ email người gửi; câu trả lời chỉ CHỨA địa chỉ thì không tính.
#[tokio::test]
async fn xoa_theo_email_gap_ca_cau_tra_loi_khac() {
    TestDatabase::run(async |test| {
        let (service, _, database) = service(test, FormsConfig::default()).await;
        let input = FormInput {
            key: Some("refer".to_owned()),
            name: Some("Refer a friend".to_owned()),
            fields: Some(fields(json!([
                {"key": "email", "type": "email", "label": {"en": "Your email"}, "required": true},
                {"key": "friend", "type": "email", "label": {"en": "Friend's email"}},
                {"key": "note", "type": "text", "label": {"en": "Note"}}
            ]))),
            reply_to_field: Some(Some("email".to_owned())),
            ..FormInput::default()
        };
        service.create_form(&admin(), input).await.unwrap();
        let client = visitor("203.0.113.9");
        let submit = |email: &str, friend: &str, note: &str| {
            let data = json!({"email": email, "friend": friend, "note": note});
            let service = service.clone();
            let client = client.clone();
            async move {
                let input = PublicSubmission {
                    data: data.as_object().unwrap().clone(),
                    form_token: Some(token(&service, "refer").await),
                    ..PublicSubmission::default()
                };
                service.submit("refer", None, &client, input).await.unwrap()
            }
        };
        submit("bob@example.com", "Ada@Example.com", "hi").await;
        submit("ada@example.com", "carl@example.com", "me").await;
        submit("dan@example.com", "erin@example.com", "write to ada@example.com.org").await;

        let exporter = user(&["export"]);
        assert_eq!(
            service.submissions_of_email(&exporter, "ada@example.com").await.unwrap().len(),
            2
        );
        assert_eq!(service.erase_email(&exporter, "ADA@example.com").await.unwrap(), 2);
        assert_eq!(count(&database).await, 1);
    })
    .await
    .unwrap();
}

/// Bài mà form không còn (không có foreign key) bị job dọn dẹp xoá; chạy lại không còn gì.
#[tokio::test]
async fn don_dep_xoa_bai_mo_coi() {
    TestDatabase::run(async |test| {
        let (service, _, database) = service(test, FormsConfig::default()).await;
        let gone = service.create_form(&admin(), contact("gone")).await.unwrap();
        service.create_form(&admin(), contact("kept")).await.unwrap();
        let client = visitor("203.0.113.9");
        send(&service, "gone", &client, answers("A", "a@example.com")).await.unwrap();
        send(&service, "kept", &client, answers("B", "b@example.com")).await.unwrap();
        // Xoá dòng form mà không qua service (như form bị xoá giữa hai pha của bài gửi).
        crate::entity::form::Entity::delete_by_id(gone.row.id)
            .exec(database.write())
            .await
            .unwrap();

        let now = chrono::Utc::now();
        assert_eq!(service.cleanup(now).await.unwrap().orphans, 1);
        assert_eq!(service.cleanup(now).await.unwrap(), crate::retention::Cleaned::default());
        assert_eq!(count(&database).await, 1);
    })
    .await
    .unwrap();
}
