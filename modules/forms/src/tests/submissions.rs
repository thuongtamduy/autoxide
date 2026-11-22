//! Gửi bài: chống spam hai pha, gửi lại cùng token, form đóng và `Origin`, event không mang câu
//! trả lời, mail báo.

use super::*;

/// Bẫy, quá nhanh, quá nhiều lần: người gửi nhận đúng câu trả lời như bài được nhận.
#[tokio::test]
async fn bay_qua_nhanh_qua_nhieu_tra_loi_giong_bai_nhan() {
    TestDatabase::run(async |test| {
        let config = FormsConfig { per_ip_per_hour: 2, ..FormsConfig::default() };
        let (service, _, database) = service(test, config.clone()).await;
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        service.create_form(&tenant, &admin(), contact("contact")).await.unwrap();
        let client = visitor("203.0.113.9");
        let accepted =
            send(&tenant, &service, "contact", &client, answers("Ada", "ada@example.com"))
                .await
                .unwrap();
        assert_eq!(count(&database).await, 1);

        // Bẫy: không lưu gì, cùng câu trả lời.
        let trapped = PublicSubmission {
            data: answers("Bot", "bot@example.com"),
            locale: Some("vi".to_owned()),
            form_token: Some(token(&tenant, &service, "contact").await),
            trap: Some("http://spam.example".to_owned()),
            ..PublicSubmission::default()
        };
        let trapped = service.submit(&tenant, "contact", None, &client, trapped).await.unwrap();
        assert_eq!(trapped.message, accepted.message);
        assert_ne!(trapped.id, accepted.id);
        assert_eq!(trapped.id.get_version_num(), 7, "a fake id looks real");
        assert_eq!(count(&database).await, 1);

        // Quá nhiều lần từ một khối IP (giới hạn 2/giờ): lần thứ ba bị bỏ, cùng câu trả lời.
        let second = send(&tenant, &service, "contact", &client, answers("Bob", "bob@example.com"))
            .await
            .unwrap();
        let third = send(
            &tenant,
            &service,
            "contact",
            &visitor("203.0.113.77"),
            answers("Cy", "cy@example.com"),
        )
        .await
        .unwrap();
        assert_eq!((second.message.as_str(), third.message.as_str()), ("Cảm ơn!", "Cảm ơn!"));
        assert_eq!(count(&database).await, 2);

        // Quá nhanh (token vừa phát chưa đủ 60 giây): lưu ở `spam`, cùng câu trả lời, không event.
        let slow = AntispamConfig::default().min_fill(Duration::from_secs(60));
        let (strict, mailer, database) = service_with(test, config, slow).await;
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let fast = send(
            &tenant,
            &strict,
            "contact",
            &visitor("198.51.100.1"),
            answers("Dee", "dee@example.com"),
        )
        .await
        .unwrap();
        assert_eq!(fast.message, accepted.message);
        let held = strict.get_submission(&tenant, &admin(), fast.id).await.unwrap();
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
            .list_submissions(&tenant, &admin(), form_id, &InboxFilter::default(), &page())
            .await
            .unwrap();
        assert_eq!(inbox.data.len(), 2);
        let spam = InboxFilter { status: Some(SubmissionStatus::Spam), search: None };
        assert_eq!(
            strict
                .list_submissions(&tenant, &admin(), form_id, &spam, &page())
                .await
                .unwrap()
                .data
                .len(),
            1
        );
        assert_eq!(strict.notify(&tenant, fast.id).await.unwrap(), 0, "no mail for spam");
        assert!(mailer.sent().is_empty());

        // Token thiếu, sai: lỗi rõ (người thật tải lại form là xong).
        let missing =
            PublicSubmission { data: answers("E", "e@example.com"), ..PublicSubmission::default() };
        assert_eq!(
            service.submit(&tenant, "contact", None, &client, missing).await.unwrap_err().code(),
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
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        service.create_form(&tenant, &admin(), contact("contact")).await.unwrap();
        let client = visitor("203.0.113.9");
        let input = |token: &str| PublicSubmission {
            data: answers("Ada", "ada@example.com"),
            form_token: Some(token.to_owned()),
            ..PublicSubmission::default()
        };
        let same = token(&tenant, &service, "contact").await;
        let first = service.submit(&tenant, "contact", None, &client, input(&same)).await.unwrap();
        let again = service.submit(&tenant, "contact", None, &client, input(&same)).await.unwrap();
        assert_eq!(first.id, again.id);
        assert_eq!(count(&database).await, 1);

        let shared = token(&tenant, &service, "contact").await;
        let mut tasks = Vec::new();
        for _ in 0..5 {
            let (service, client, input) = (service.clone(), client.clone(), input(&shared));
            let tenant = tenant.clone();
            tasks.push(tokio::spawn(async move {
                service.submit(&tenant, "contact", None, &client, input).await
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
        let (service, _, database) = service(test, config).await;
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let mut input = contact("contact");
        input.allowed_origins = Some(vec!["https://Example.com/".to_owned()]);
        let form = service.create_form(&tenant, &admin(), input).await.unwrap();
        assert_eq!(form.allowed_origins, ["https://example.com"]);

        let mut client = visitor("203.0.113.9");
        client.origin = Some("https://evil.example".to_owned());
        let refused = send(&tenant, &service, "contact", &client, answers("A", "a@example.com"))
            .await
            .unwrap_err();
        assert_eq!(
            (refused.kind(), refused.code()),
            (ErrorKind::Forbidden, "forms.origin_not_allowed")
        );
        client.origin = Some("https://example.com".to_owned());
        send(&tenant, &service, "contact", &client, answers("A", "a@example.com")).await.unwrap();
        // Không có `Origin` (máy chủ gọi): không kiểm.
        send(&tenant, &service, "contact", &visitor("203.0.113.9"), answers("B", "b@example.com"))
            .await
            .unwrap();

        // Đăng nhập mà không có `forms.submit` (tài khoản tự đăng ký): đi đường của người lạ —
        // thiếu token thì bị từ chối như người lạ; có token thì được nhận.
        let member = Principal::new(Uuid::now_v7(), Vec::<String>::new());
        let signed = || PublicSubmission {
            data: answers("C", "c@example.com"),
            ..PublicSubmission::default()
        };
        let refused = service.submit(&tenant, "contact", Some(&member), &client, signed()).await;
        assert_eq!(refused.unwrap_err().code(), "antispam.token_missing");
        let mut with_token = signed();
        with_token.form_token = Some(token(&tenant, &service, "contact").await);
        service.submit(&tenant, "contact", Some(&member), &client, with_token).await.unwrap();

        // Website gọi bằng API key có `forms.submit`: không cần token; vượt giới hạn theo người
        // dùng thì 429.
        let website = user(&["submit"]);
        service.submit(&tenant, "contact", Some(&website), &client, signed()).await.unwrap();
        let limited = service
            .submit(&tenant, "contact", Some(&website), &client, signed())
            .await
            .unwrap_err();
        assert_eq!(limited.code(), "antispam.rate_limited");

        let closed = FormInput { status: Some(FormStatus::Closed), ..FormInput::default() };
        service.update_form(&tenant, &admin(), form.row.id, closed, None).await.unwrap();
        let error = send(&tenant, &service, "contact", &client, answers("D", "d@example.com"))
            .await
            .unwrap_err();
        assert_eq!((error.kind(), error.code()), (ErrorKind::Conflict, "forms.closed"));
        assert_eq!(
            send(&tenant, &service, "nope", &client, answers("D", "d@example.com"))
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
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let form = service.create_form(&tenant, &admin(), contact("contact")).await.unwrap();
        let receipt = send(
            &tenant,
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
        let delivered = Delivered::new(
            tenant.clone(),
            "forms",
            "forms.submission_received".to_owned(),
            payload,
        )
        .actor(Actor::public());
        let notifier = crate::submissions::notify::Notifier(service.clone());
        notifier.handle(&delivered).await.unwrap();
        notifier.handle(&delivered).await.unwrap();
        // Hai lời gọi, một mail: lần sau mang cùng `dedupe_key` nên bị bỏ như ở module `mail`.
        assert_eq!(mailer.calls(), 2);
        let sent = mailer.sent();
        assert_eq!(sent.len(), 1);
        assert!(sent[0].dedupe_key.is_some());
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
        let current = service.get_submission(&tenant, &admin(), receipt.id).await.unwrap();
        let reviewer = user(&["review"]);
        let spam = service
            .set_submission_status(
                &tenant,
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
                    &tenant,
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
                &tenant,
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
                .set_submission_status(
                    &tenant,
                    &user(&["read"]),
                    receipt.id,
                    SubmissionStatus::Read,
                    None
                )
                .await
                .unwrap_err()
                .code(),
            "permission_denied"
        );

        service.delete_submission(&tenant, &reviewer, receipt.id).await.unwrap();
        assert_eq!(service.notify(&tenant, receipt.id).await.unwrap(), 0, "deleted since: no mail");
        assert_eq!(
            service.get_submission(&tenant, &admin(), receipt.id).await.unwrap_err().code(),
            "forms.submission_not_found"
        );
        let _ = form;
    })
    .await
    .unwrap();
}

/// Bên gửi mail không nhận địa chỉ người gửi làm `Reply-To`: mail báo vẫn đi, không có
/// `Reply-To`, event không lỗi (không bị thử lại tới khi hỏng hẳn).
#[tokio::test]
async fn mail_bao_bo_reply_to_khi_ben_gui_tu_choi() {
    TestDatabase::run(async |test| {
        let (service, mailer, database) = service(test, FormsConfig::default()).await;
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        service.create_form(&tenant, &admin(), contact("contact")).await.unwrap();
        let receipt = send(
            &tenant,
            &service,
            "contact",
            &visitor("203.0.113.9"),
            answers("Ada", "ada@example.com"),
        )
        .await
        .unwrap();
        mailer.reject_reply_to(true);
        assert_eq!(service.notify(&tenant, receipt.id).await.unwrap(), 1);
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
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        service.create_form(&tenant, &admin(), contact("contact")).await.unwrap();
        let client = visitor("203.0.113.9");
        let long = format!("{}@{}.example.com", "İ".repeat(30), "x".repeat(60));
        let error =
            send(&tenant, &service, "contact", &client, answers("A", &long)).await.unwrap_err();
        assert_eq!(error.code(), "validation_failed");
        assert_eq!(error.fields()[0].field, "data.email");

        let fits = format!("{}@{}.example.com", "İ".repeat(20), "x".repeat(60));
        let receipt =
            send(&tenant, &service, "contact", &client, answers("B", &fits)).await.unwrap();
        let stored = service.get_submission(&tenant, &admin(), receipt.id).await.unwrap();
        assert_eq!(stored.row.email, Some(fits.to_lowercase()));
        assert_eq!(count(&database).await, 1);
    })
    .await
    .unwrap();
}
