//! Dữ liệu đã gửi: CSV, xuất / xoá theo email, dọn theo thời hạn, tài khoản bị xoá.

use super::*;
use autoxide_kernel::tenancy::TenantContext;

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
        let exported = service
            .export(&TenantContext::legacy_default(&service.database), owner.user_id())
            .await
            .unwrap();
        assert_eq!(exported["forms"][0]["key"], "contact");

        let deleted = Delivered::new(
            autoxide_kernel::tenancy::TenantContext::legacy_default(&service.database),
            "auth",
            USER_DELETED.to_owned(),
            json!({"before": {"id": owner.user_id()}}),
        );
        service.handle(&deleted).await.unwrap();
        service.handle(&deleted).await.unwrap();
        assert_eq!(service.get_form(&owner, form.row.id).await.unwrap().row.created_by, None);
        assert_eq!(
            service
                .export(&TenantContext::legacy_default(&service.database), owner.user_id())
                .await
                .unwrap()["forms"],
            json!([])
        );
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
