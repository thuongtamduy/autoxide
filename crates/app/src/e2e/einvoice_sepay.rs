//! Bộ nối SePay eInvoice với `invoices` thật (bước 70 P7; cả SQLite và Postgres): chứng từ phát hành
//! được gửi tới máy chủ giả của SePay đúng như tài liệu, hỏi trạng thái tới khi cơ quan thuế chấp
//! nhận, rồi tải file — bằng tài khoản SePay do chính khách đặt.

use autoxide_kernel::event::Actor;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgTree, root_of};
use autoxide_kernel::time::Timestamp;
use autoxide_testkit::{FakeProviderServer, TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

use autoxide_invoices::api::{Buyer, Invoices, NewDocument, NewDocumentLine};

const PERMISSIONS: [&str; 6] = [
    "invoices.read",
    "invoices.write",
    "invoices.issue",
    "invoices.series_manage",
    "tax.packs_manage",
    "kernel.settings.manage",
];
const ACCOUNT: &str = "0aea3134-da40-11f0-aef4-52c7e9b4f41b";

async fn call(
    app: &TestApp,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = app.request(method, uri).token("clerk");
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let body = if answer.body().is_empty() { Value::Null } else { answer.json() };
    (answer.status(), body)
}

async fn drain(app: &TestApp) {
    let worker = app.modules().worker(4).unwrap().blocked_delay(std::time::Duration::ZERO);
    for _ in 0..5 {
        if worker.run_once().await.unwrap() == 0 {
            break;
        }
    }
}

#[tokio::test]
async fn chung_tu_phat_hanh_duoc_gui_qua_sepay_va_duoc_chap_nhan() {
    TestDatabase::run(async |test| {
        let server = FakeProviderServer::start().await.unwrap();
        let mut builder = TestApp::builder(test)
            .setting("AUTOXIDE_EINVOICE_SEPAY_API_URL", server.url())
            .user("clerk", 1, &PERMISSIONS);
        for module in crate::modules::all() {
            builder = builder.boxed_module(module);
        }
        let app = builder.build().await.unwrap();
        let write = app.database().begin_write().await.unwrap();
        // Module `organizations` có thể đã tạo nút gốc.
        let _ = OrgTree::insert_root(&write, DEFAULT_TENANT).await;
        write.commit().await.unwrap();
        let pack: Value =
            serde_json::from_str(include_str!("../../../../modules/tax/packs/vn.json")).unwrap();
        let (status, body) = call(&app, Method::POST, "/api/v1/tax/packs", Some(json!({"pack": pack}))).await;
        assert_eq!(status, StatusCode::OK, "{body}");

        // Khách đặt tài khoản SePay eInvoice của mình.
        for (field, value) in [("client_id", "id-one"), ("client_secret", "secret-one"), ("provider_account_id", ACCOUNT)] {
            let uri = format!("/api/v1/kernel/settings/einvoice_sepay.connection.api.{field}");
            let (status, body) = call(&app, Method::PUT, &uri, Some(json!({"value": value}))).await;
            assert_eq!(status, StatusCode::OK, "{body}");
        }
        let year = i32::from(Timestamp::now().date_utc().year());
        let (status, body) = call(
            &app,
            Method::POST,
            "/api/v1/invoices/series",
            Some(json!({"kind": "vat_invoice", "symbol": "1C26TAA", "year": year, "einvoice_provider": "sepay"})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");

        let line = NewDocumentLine::new("Service fee", rust_decimal::Decimal::ONE, Money::new(200_000, Currency::VND))
            .service()
            .tax_category("vat.standard");
        let document = NewDocument::new(
            root_of(DEFAULT_TENANT),
            "vat_invoice",
            Currency::VND,
            Buyer::business("Buyer Co", "0101234567"),
            vec![line],
            "demo",
            "R-1",
        );
        let invoices = app.api::<dyn Invoices>().unwrap();
        let found = invoices
            .create_document(&app.default_tenant(), Actor::system(), document)
            .await
            .unwrap()
            .into_inner();

        server.reply("POST", "/v1/token", 200, json!({"success": true, "data": {"access_token": "token-one", "token_type": "Bearer", "expires_in": 86400}}));
        server.reply("GET", &format!("/v1/provider-accounts/{ACCOUNT}"), 200, json!({"success": true, "data": {"id": ACCOUNT, "active": true,
            "templates": [{"template_code": "1", "invoice_series": "C26TSE", "invoice_label": "1 - C26TSE"}]}}));
        server.reply("POST", "/v1/invoices/create", 200, json!({"success": true, "data": {"tracking_code": "trk-1",
            "tracking_url": "https://einvoice-api.sepay.vn/v1/invoices/create/check/trk-1"}}));
        // Lần hỏi đầu: SePay còn xử lý; lần sau: đã phát hành.
        server.reply("GET", "/v1/invoices/create/check/trk-1", 200, json!({"success": true, "data": {"status": "Pending", "message": "Đang xử lý"}}));
        server.reply("GET", "/v1/invoices/create/check/trk-1", 200, json!({"success": true, "data": {"status": "Success", "message": "Thành công",
            "invoice": {"reference_code": found.id.to_string(), "invoice_number": "589", "status": "issued"}}}));
        server.reply("GET", &format!("/v1/invoices/{}", found.id), 200, json!({"success": true, "data": {"reference_code": found.id.to_string(),
            "invoice_number": "589", "status": "issued", "provider_status": "4", "tax_authority_code": "M1-26-ABCDE-00000000589"}}));
        server.reply("GET", "/v1/invoices/trk-1/download", 200, json!({"success": true, "data": {"file_type": "pdf", "file_name": "HD.pdf", "content": "JVBERi0xLjQK"}}));

        let issue = format!("/api/v1/invoices/documents/{}/issue", found.id);
        let (status, body) = call(&app, Method::POST, &issue, None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        drain(&app).await;
        let detail = format!("/api/v1/invoices/documents/{}", found.id);
        // Job gửi hỏi trạng thái ngay một lần (SePay còn xử lý); các lượt hỏi sau có thể đã chạy.
        let (_, body) = call(&app, Method::GET, &detail, None).await;
        assert!(["submitting", "accepted"].contains(&body["data"]["status"].as_str().unwrap()), "{body}");

        // Thân gửi đi đúng tài liệu: mã tham chiếu là ID chứng từ, ký hiệu của SePay, phát hành luôn.
        let seen = server.take_requests();
        let created = seen.iter().find(|request| request.path == "/v1/invoices/create").unwrap();
        assert_eq!(created.header("authorization"), Some("Bearer token-one"));
        let sent = created.json();
        assert_eq!(sent["reference_code"], found.id.to_string());
        assert_eq!((&sent["template_code"], &sent["invoice_series"], &sent["is_draft"]), (&json!("1"), &json!("C26TSE"), &json!(false)));
        assert_eq!(sent["provider_account_id"], ACCOUNT);
        assert_eq!(sent["buyer"]["tax_code"], "0101234567");
        assert_eq!(sent["items"][0]["unit_price"], 200_000);
        assert_eq!(sent["currency"], "VND");

        // Hỏi lại: cơ quan thuế đã cấp mã.
        let refresh = format!("/api/v1/invoices/documents/{}/refresh", found.id);
        let (status, body) = call(&app, Method::POST, &refresh, None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["data"]["status"], "accepted", "{body}");
        assert_eq!(body["data"]["authority_code"], "M1-26-ABCDE-00000000589");
        // Số do nhà cung cấp cấp (số hợp lệ về thuế) được lưu và trả ra cùng mã cơ quan thuế.
        assert_eq!(body["data"]["provider_number"], 589, "{body}");

        let file = app
            .get(&format!("/api/v1/invoices/documents/{}/einvoice?format=pdf", found.id))
            .token("clerk")
            .send()
            .await
            .unwrap();
        assert_eq!(file.status(), StatusCode::OK);
        assert!(file.body().starts_with(b"%PDF-1.4"));
        // Client secret không xuất hiện ở đâu trong câu trả lời của API cài đặt.
        let (_, listed) = call(&app, Method::GET, "/api/v1/kernel/settings?group=connection", None).await;
        assert!(!listed.to_string().contains("secret-one"), "{listed}");
    })
    .await
    .unwrap();
}
