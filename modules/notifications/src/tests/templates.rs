//! Mẫu khách tự soạn (D-206, D-365), cả SQLite và Postgres: kiểm biến theo schema khi lưu (gom lỗi một
//! lần), nháp → xuất bản → quay lại, giá trị chèn vào HTML luôn được thoát, mail HTML trong khung MJML
//! có thương hiệu kèm bản chữ thuần, tin quảng cáo luôn có chân thư huỷ nhận, gửi thử chỉ tới chính
//! người gọi, mẫu hỏng lúc gửi thì dùng mẫu mặc định, giới hạn chạy, permission kiểm trước.

use super::*;
use autoxide_kernel::template::Variable;

const RESET: NotificationKind = NotificationKind::new(
    Category::Security,
    &[Channel::Email],
    template("shop.password_reset", "Reset", "Open {link}", "Reset"),
)
.variables(&[
    Variable::required("link", "Reset link."),
    Variable::optional("customer.name", "Name of the customer."),
])
.sample(r#"{"link": "https://shop.example/reset/abc", "customer": {"name": "An"}}"#);

const EMAIL_VI: &str = "/templates/shop.order_shipped/email/vi";

async fn templates_fixture(test: &TestDatabase) -> Fixture {
    let fx = fixture(test).await;
    fx.notifier.declare(RESET).unwrap();
    fx
}

#[tokio::test]
async fn luu_mau_kiem_bien_theo_schema_va_gom_loi_mot_lan() {
    TestDatabase::run(async |test| {
        let fx = templates_fixture(test).await;
        // Permission trước mọi kiểm tra (403 trước 422).
        let bad = json!({"subject": "{{ secret }}", "body_text": "{% if %}"});
        let (status, _) =
            call(&fx.app, "PUT", &format!("{EMAIL_VI}/draft"), "alice", Some(bad.clone())).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, problem) =
            call(&fx.app, "PUT", &format!("{EMAIL_VI}/draft"), "admin", Some(bad)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        let codes: Vec<&str> = problem["errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|error| error["code"].as_str().unwrap())
            .collect();
        assert!(codes.contains(&"template.unknown_variable"), "{problem}");
        assert!(codes.contains(&"template.syntax_error"), "{problem}");
        // Biến bắt buộc của loại (link đặt lại mật khẩu) phải có mặt.
        let missing = json!({"subject": "Reset", "body_text": "Hi {{ customer.name }}"});
        let (status, problem) = call(
            &fx.app,
            "PUT",
            "/templates/shop.password_reset/email/en/draft",
            "admin",
            Some(missing),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(problem["errors"][0]["code"], "template.required_variable_missing", "{problem}");
        let (status, schema) =
            call(&fx.app, "GET", "/kinds/shop.password_reset/schema", "alice", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(schema["data"]["variables"][0]["required"], true);
        assert_eq!(schema["data"]["sample"]["customer"]["name"], "An");
        let (status, schema) =
            call(&fx.app, "GET", "/kinds/shop.order_shipped/schema", "alice", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(schema["data"]["sample"]["code"], "[code]");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn xuat_ban_va_quay_lai_phien_ban_mail_html_trong_khung() {
    TestDatabase::run(async |test| {
        let fx = templates_fixture(test).await;
        let branding = json!({"brand_name": "Shop A", "logo_url": "https://cdn.example/logo.png", "primary_color": "#AA0000", "footer_text": "Shop A, 1 Street"});
        let (status, saved) = call(&fx.app, "PUT", "/branding", "admin", Some(branding)).await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        assert_eq!(saved["data"]["primary_color"], "#aa0000");
        let (status, _) = call(&fx.app, "PUT", "/branding", "admin", Some(json!({"logo_url": "http://x", "primary_color": "red"}))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

        let v1 = json!({"subject": "Đơn {{ code }} v1", "body_text": "Đơn {{ code }} đang đi.", "body_html": "<h1>Đơn {{ code }}</h1>{% if link %}<a href=\"{{ link }}\">Xem</a>{% endif %}"});
        let (status, _) = call(&fx.app, "PUT", &format!("{EMAIL_VI}/draft"), "admin", Some(v1)).await;
        assert_eq!(status, StatusCode::OK);
        // Chưa xuất bản: mẫu mặc định vẫn dùng.
        fx.notifier.notify(DEFAULT_TENANT, to_alice(&SHIPPED, "t-0").locale("vi")).await.unwrap();
        run_jobs(&fx.app).await;
        assert_eq!(fx.mails.all()[0].subject, "Order DH001 shipped");
        // Mail mặc định cũng có HTML trong khung có thương hiệu.
        assert!(fx.mails.all()[0].html.as_deref().unwrap().contains("Shop A, 1 Street"));

        let (status, published) = call(&fx.app, "POST", &format!("{EMAIL_VI}/publish"), "admin", None).await;
        assert_eq!(status, StatusCode::OK, "{published}");
        assert_eq!(published["data"]["version"], 1);
        let (status, _) = call(&fx.app, "GET", &format!("{EMAIL_VI}/draft"), "admin", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = call(&fx.app, "POST", &format!("{EMAIL_VI}/publish"), "admin", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // Giá trị chèn vào HTML luôn được thoát.
        let hostile = Notification::new(SHIPPED.key(), Recipient::User(Uuid::from_u128(2)), json!({"code": "<script>x</script>"}), "t-1")
            .locale("vi")
            .link("https://shop.example/o/1");
        fx.notifier.notify(DEFAULT_TENANT, hostile).await.unwrap();
        run_jobs(&fx.app).await;
        let mail = fx.mails.all().pop().unwrap();
        assert_eq!(mail.subject, "Đơn <script>x</script> v1");
        assert!(mail.text.contains("Đơn <script>x</script> đang đi."), "{}", mail.text);
        let html = mail.html.unwrap();
        assert!(!html.contains("<script>"), "{html}");
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("<a href=\"https:&#x2f;&#x2f;shop.example&#x2f;o&#x2f;1\">Xem</a>"), "{html}");
        assert!(html.contains("#aa0000") && html.contains("https://cdn.example/logo.png"));

        let v2 = json!({"subject": "Đơn {{ code }} v2", "body_text": "v2"});
        call(&fx.app, "PUT", &format!("{EMAIL_VI}/draft"), "admin", Some(v2)).await;
        let (_, published) = call(&fx.app, "POST", &format!("{EMAIL_VI}/publish"), "admin", None).await;
        assert_eq!(published["data"]["version"], 2);
        let (_, versions) = call(&fx.app, "GET", &format!("{EMAIL_VI}/versions"), "admin", None).await;
        let list = versions["data"].as_array().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!((list[0]["version"].as_i64(), list[0]["live"].as_bool()), (Some(2), Some(true)));
        // Bản đã xuất bản không đổi: quay lại v1 dùng đúng nội dung cũ.
        let (status, rolled) = call(&fx.app, "POST", &format!("{EMAIL_VI}/rollback"), "admin", Some(json!({"version": 1}))).await;
        assert_eq!(status, StatusCode::OK, "{rolled}");
        assert_eq!(rolled["data"]["subject"], "Đơn {{ code }} v1");
        let (status, _) = call(&fx.app, "POST", &format!("{EMAIL_VI}/rollback"), "admin", Some(json!({"version": 9}))).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        fx.notifier.notify(DEFAULT_TENANT, to_alice(&SHIPPED, "t-2").locale("vi")).await.unwrap();
        run_jobs(&fx.app).await;
        assert_eq!(fx.mails.all().pop().unwrap().subject, "Đơn DH001 v1");
        // Thôi dùng: mẫu mặc định áp lại; các bản vẫn còn.
        let (status, _) = call(&fx.app, "DELETE", &format!("{EMAIL_VI}/published"), "admin", None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _) = call(&fx.app, "DELETE", &format!("{EMAIL_VI}/published"), "admin", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        fx.notifier.notify(DEFAULT_TENANT, to_alice(&SHIPPED, "t-3").locale("vi")).await.unwrap();
        run_jobs(&fx.app).await;
        assert_eq!(fx.mails.all().pop().unwrap().subject, "Order DH001 shipped");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn mau_quang_cao_luon_co_chan_thu_huy_nhan_va_mau_hong_dung_mac_dinh() {
    TestDatabase::run(async |test| {
        let fx = templates_fixture(test).await;
        fx.ledger.allow(ConsentSubject::User(Uuid::from_u128(2)), "");
        let promo =
            json!({"subject": "Giảm {{ percent }}%", "body_text": "Mọi thứ giảm {{ percent }}%."});
        call(&fx.app, "PUT", "/templates/shop.promo/email/en/draft", "admin", Some(promo)).await;
        let (status, _) =
            call(&fx.app, "POST", "/templates/shop.promo/email/en/publish", "admin", None).await;
        assert_eq!(status, StatusCode::OK);
        fx.notifier.notify(DEFAULT_TENANT, to_alice(&PROMO, "p-1")).await.unwrap();
        run_jobs(&fx.app).await;
        let mail = fx.mails.all().pop().unwrap();
        assert_eq!(mail.subject, "[QC] Giảm 30%");
        assert!(mail.text.contains("https://shop.example/unsubscribe?token="), "{}", mail.text);
        assert!(mail.html.as_deref().unwrap().contains("https://shop.example/unsubscribe?token="));

        // Mẫu lưu được (cú pháp, biến đúng) nhưng không dựng được với dữ liệu thật → mẫu mặc định,
        // tin vẫn gửi, ghi `template_fallback`.
        let broken = json!({"subject": "{{ code // 0 }}", "body_text": "x"});
        let (status, saved) =
            call(&fx.app, "PUT", &format!("{EMAIL_VI}/draft"), "admin", Some(broken)).await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        call(&fx.app, "POST", &format!("{EMAIL_VI}/publish"), "admin", None).await;
        fx.notifier.notify(DEFAULT_TENANT, to_alice(&SHIPPED, "b-1").locale("vi")).await.unwrap();
        run_jobs(&fx.app).await;
        assert_eq!(fx.mails.all().pop().unwrap().subject, "Order DH001 shipped");
        let (_, page) = call(&fx.app, "GET", "/messages?limit=200", "admin", None).await;
        let row = page["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["dedupe_key"] == "b-1" && row["channel"] == "email")
            .unwrap()
            .clone();
        assert_eq!(
            (row["status"].as_str(), row["error_code"].as_str()),
            (Some("sent"), Some("template_fallback")),
            "{row}"
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn xem_truoc_gui_thu_chi_toi_nguoi_goi_va_gioi_han_chay() {
    TestDatabase::run(async |test| {
        let fx = templates_fixture(test).await;
        let draft = json!({"subject": "Hi {{ customer.name }}", "body_text": "Reset: {{ link }}", "body_html": "<p>{{ customer.name }}</p><a href=\"{{ link }}\">Reset</a>"});
        let path = "/templates/shop.password_reset/email/en";
        let (status, _) = call(&fx.app, "PUT", &format!("{path}/draft"), "admin", Some(draft)).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = call(&fx.app, "POST", &format!("{path}/preview"), "alice", Some(json!({}))).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, preview) = call(&fx.app, "POST", &format!("{path}/preview"), "admin", Some(json!({}))).await;
        assert_eq!(status, StatusCode::OK, "{preview}");
        assert_eq!(preview["data"]["subject"], "Hi An");
        assert!(preview["data"]["html"].as_str().unwrap().contains("<p>An</p>"));
        let (status, preview) = call(&fx.app, "POST", &format!("{path}/preview"), "admin", Some(json!({"data": {"customer": {"name": "<b>B</b>"}, "link": "x"}}))).await;
        assert_eq!(status, StatusCode::OK);
        assert!(preview["data"]["html"].as_str().unwrap().contains("<p>&lt;b&gt;B&lt;&#x2f;b&gt;</p>"));
        let (status, problem) = call(&fx.app, "POST", &format!("{path}/preview"), "admin", Some(json!({"data": {"password": "x"}}))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(problem["errors"][0]["code"], "template.unknown_variable");
        let (status, problem) = call(&fx.app, "POST", &format!("{path}/preview"), "admin", Some(json!({"source": "published"}))).await;
        assert_eq!((status, problem["code"].as_str()), (StatusCode::NOT_FOUND, Some("notifications.template_not_published")));

        // Gửi thử: chỉ tới email đã xác minh của chính người gọi, không nhận địa chỉ khác.
        let (status, _) = call(&fx.app, "POST", &format!("{path}/test-send"), "admin", Some(json!({"to": "victim@example.com"}))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let (status, sent) = call(&fx.app, "POST", &format!("{path}/test-send"), "admin", Some(json!({}))).await;
        assert_eq!(status, StatusCode::OK, "{sent}");
        let mail = fx.mails.all().pop().unwrap();
        assert_eq!(mail.to, "admin@example.com");
        assert_eq!(mail.subject, "[Test] Hi An");
        assert!(mail.html.is_some());
        let (status, _) = call(&fx.app, "POST", "/templates/shop.promo/sms/en/test-send", "admin", Some(json!({}))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

        // Vòng lặp lồng nhau rất lớn bị cắt theo giới hạn, không treo.
        let looping = json!({"subject": "x", "body_text": "{% for i in range(10000) %}{% for j in range(10000) %}{{ link }}{% endfor %}{% endfor %}"});
        let (status, saved) = call(&fx.app, "PUT", &format!("{path}/draft"), "admin", Some(looping)).await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        let (status, problem) = call(&fx.app, "POST", &format!("{path}/preview"), "admin", Some(json!({}))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(problem["code"], "template.limit_exceeded", "{problem}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tool_mcp_soan_nhap_va_xem_truoc() {
    TestDatabase::run(async |test| {
        let fx = templates_fixture(test).await;
        let client = fx.app.mcp("admin").await.unwrap();
        let saved = client
            .call_tool(autoxide_testkit::tool(
                "notifications.update_template_draft",
                json!({"kind": "shop.order_shipped", "channel": "in_app", "locale": "en", "subject": "Order {{ code }}", "body_text": "On its way"}),
            ))
            .await
            .unwrap();
        assert_ne!(saved.is_error, Some(true), "{saved:?}");
        let preview = client
            .call_tool(autoxide_testkit::tool(
                "notifications.preview_template",
                json!({"kind": "shop.order_shipped", "channel": "in_app", "locale": "en", "data": {"code": "Z9"}}),
            ))
            .await
            .unwrap();
        assert!(format!("{preview:?}").contains("Order Z9"), "{preview:?}");
        let schema = client
            .call_tool(autoxide_testkit::tool("notifications.get_template_schema", json!({"kind": "shop.order_shipped"})))
            .await
            .unwrap();
        assert!(format!("{schema:?}").contains("code"), "{schema:?}");
    })
    .await
    .unwrap();
}
