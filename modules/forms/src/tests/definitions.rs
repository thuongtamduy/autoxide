//! Định nghĩa form: vòng đời và phiên bản, kiểm từng kiểu trường, đổi nơi nhận mail báo.

use super::*;

#[tokio::test]
async fn vong_doi_form_phien_ban_va_bai_cu_doc_dung_nhan_cu() {
    TestDatabase::run(async |test| {
        let (service, _, database) = service(test, FormsConfig::default()).await;
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let admin = admin();
        let form = service.create_form(&tenant, &admin, contact("contact")).await.unwrap();
        assert_eq!((form.row.version, form.status()), (1, FormStatus::Open));
        assert_eq!(form.notify_emails, ["owner@example.com"]);

        // Thiếu quyền; key trùng; định nghĩa sai báo mọi lỗi một lần.
        let reader = user(&["read"]);
        assert_eq!(
            service.create_form(&tenant, &reader, contact("other")).await.unwrap_err().code(),
            "permission_denied"
        );
        assert_eq!(
            service.create_form(&tenant, &admin, contact("contact")).await.unwrap_err().code(),
            "forms.key_taken"
        );
        let mut bad = contact("Bad Key");
        bad.name = Some(" ".to_owned());
        bad.allowed_origins = Some(vec!["example.com".to_owned()]);
        bad.reply_to_field = Some(Some("name".to_owned()));
        bad.fields = Some(fields(json!([{"key": "x", "type": "select", "label": {}}])));
        let error = service.create_form(&tenant, &admin, bad).await.unwrap_err();
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
        let first = send(&tenant, &service, "contact", &client, answers("Ada", "ada@example.com"))
            .await
            .unwrap();

        // Đổi trường: phiên bản 2; sai phiên bản thì 412.
        let mut changed = contact_fields();
        changed[0].label = Localized { en: Some("Full name".to_owned()), vi: None };
        changed.remove(2);
        let input = FormInput { fields: Some(changed), ..FormInput::default() };
        let stale = Some(form.row.updated_at - chrono::Duration::seconds(1));
        assert_eq!(
            service
                .update_form(&tenant, &admin, form.row.id, input.clone(), stale)
                .await
                .unwrap_err()
                .code(),
            "precondition_failed"
        );
        let updated = service
            .update_form(&tenant, &admin, form.row.id, input, Some(form.row.updated_at))
            .await
            .unwrap();
        assert_eq!(updated.row.version, 2);
        // Đổi thứ khác trường: không tăng phiên bản.
        let renamed = FormInput { name: Some("Liên hệ".to_owned()), ..FormInput::default() };
        assert_eq!(
            service
                .update_form(&tenant, &admin, form.row.id, renamed, None)
                .await
                .unwrap()
                .row
                .version,
            2
        );

        // Bài cũ vẫn đọc với nhãn và trường của phiên bản 1.
        let old = service.get_submission(&tenant, &admin, first.id).await.unwrap();
        assert_eq!((old.row.form_version, old.fields.len()), (1, 4));
        assert_eq!(old.fields[0].label.en.as_deref(), Some("Name"));
        assert_eq!(old.data["message"], "Hello there");
        // Bài mới theo phiên bản 2: trường `message` không còn nhận.
        let mut later = answers("Bob", "bob@example.com");
        assert_eq!(
            send(&tenant, &service, "contact", &client, later.clone()).await.unwrap_err().fields()
                [0]
            .field,
            "data.message"
        );
        later.remove("message");
        let second = send(&tenant, &service, "contact", &client, later).await.unwrap();
        assert_eq!(
            service.get_submission(&tenant, &admin, second.id).await.unwrap().fields[0]
                .label
                .en
                .as_deref(),
            Some("Full name")
        );

        let listed = service.list_forms(&tenant, &reader, None, &page()).await.unwrap();
        assert_eq!(listed.data.len(), 1);
        // Xoá form: mất cả bài gửi.
        let current = service.get_form(&tenant, &admin, form.row.id).await.unwrap();
        service
            .delete_form(&tenant, &admin, form.row.id, Some(current.row.updated_at))
            .await
            .unwrap();
        assert_eq!(count(&database).await, 0);
        assert_eq!(
            service.get_form(&tenant, &admin, form.row.id).await.unwrap_err().code(),
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
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
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
        service.create_form(&tenant, &admin(), input).await.unwrap();
        let client = visitor("203.0.113.9");
        let wrong = json!({
            "name": "much too long", "email": "nope", "phone": "abc", "note": "no",
            "guests": 0, "topic": "b", "size": ["s"], "extras": ["z"], "day": "12/31/2026",
            "source": 7, "agree": "yes", "hacker": "?"
        });
        let error = send(&tenant, &service, "survey", &client, wrong.as_object().unwrap().clone()).await.unwrap_err();
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
        let receipt = send(&tenant, &service, "survey", &client, right.as_object().unwrap().clone()).await.unwrap();
        assert_eq!(receipt.message, "Cảm ơn!");
        let stored = service.get_submission(&tenant, &admin(), receipt.id).await.unwrap();
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

/// `forms.manage` thôi thì không đặt / đổi được `notify_emails` (mail báo chứa câu trả lời):
/// cần thêm `forms.read`. Gửi lại đúng danh sách cũ thì không cần.
#[tokio::test]
async fn doi_noi_nhan_mail_bao_can_quyen_doc() {
    TestDatabase::run(async |test| {
        let (service, _, database) = service(test, FormsConfig::default()).await;
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let manager = user(&["manage"]);
        assert_eq!(
            service.create_form(&tenant, &manager, contact("contact")).await.unwrap_err().code(),
            "permission_denied"
        );
        let mut quiet = contact("contact");
        quiet.notify_emails = None;
        let form = service.create_form(&tenant, &manager, quiet).await.unwrap();
        assert!(form.notify_emails.is_empty());

        let redirect = || FormInput {
            notify_emails: Some(vec!["thief@example.com".to_owned()]),
            ..FormInput::default()
        };
        let refused = service.update_form(&tenant, &manager, form.row.id, redirect(), None).await;
        assert_eq!(refused.unwrap_err().code(), "permission_denied");
        let rename = FormInput {
            name: Some("Contact".to_owned()),
            notify_emails: Some(Vec::new()),
            ..FormInput::default()
        };
        service.update_form(&tenant, &manager, form.row.id, rename, None).await.unwrap();

        let both = user(&["manage", "read"]);
        let changed =
            service.update_form(&tenant, &both, form.row.id, redirect(), None).await.unwrap();
        assert_eq!(changed.notify_emails, ["thief@example.com"]);
        let same = FormInput {
            notify_emails: Some(vec!["Thief@Example.com".to_owned()]),
            ..FormInput::default()
        };
        service.update_form(&tenant, &manager, form.row.id, same, None).await.unwrap();
        let removed = FormInput { notify_emails: Some(Vec::new()), ..FormInput::default() };
        let refused = service.update_form(&tenant, &manager, form.row.id, removed, None).await;
        assert_eq!(refused.unwrap_err().code(), "permission_denied");
    })
    .await
    .unwrap();
}
