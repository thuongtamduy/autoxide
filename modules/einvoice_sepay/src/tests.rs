//! Test bộ nối SePay eInvoice với máy chủ giả (cả SQLite và Postgres): token theo từng khách, thân
//! request đúng tài liệu, gửi lại không tạo hoá đơn thứ hai, trạng thái, tải file, thay thế và
//! điều chỉnh, khách chưa cấu hình.

use autoxide_kernel::connections::ConnectionStatus;
use autoxide_kernel::einvoice::{
    EInvoice, EInvoiceCancel, EInvoiceFormat, EInvoiceKind, EInvoiceLine, EInvoiceProvider,
    EInvoiceStatus, EInvoiceTaxTotal, Party, RelatedInvoice, Relation,
};
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::provider::ProviderAccounts;
use autoxide_kernel::time::Date;
use autoxide_testkit::{FakeProviderServer, TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::client::Client;
use crate::config::EInvoiceConfig;
use crate::provider::SepayEInvoice;

const ACCOUNT: &str = "0aea3134-da40-11f0-aef4-52c7e9b4f41b";

async fn app(test: &TestDatabase, server: &FakeProviderServer) -> TestApp {
    TestApp::builder(test)
        .setting("AUTOXIDE_EINVOICE_SEPAY_API_URL", server.url())
        .tenant("one")
        .tenant("two")
        .user_in("one", "one", 1, &["kernel.settings.manage"])
        .user_in("two", "two", 2, &["kernel.settings.manage"])
        .module(crate::module())
        .build()
        .await
        .unwrap()
}

async fn set(app: &TestApp, token: &str, field: &str, value: &str) {
    let uri = format!("/api/v1/kernel/settings/einvoice_sepay.connection.api.{field}");
    let answer = app.put(&uri).token(token).json(json!({"value": value})).send().await.unwrap();
    assert_eq!(answer.status(), StatusCode::OK, "{}", answer.text());
}

fn provider(app: &TestApp, server: &FakeProviderServer) -> SepayEInvoice {
    let accounts: ProviderAccounts = (*app.api::<ProviderAccounts>().unwrap()).clone();
    let config = EInvoiceConfig {
        api_url: server.url().to_owned(),
        // Khách đặt `environment = sandbox` gọi sang đường `/sandbox` của máy chủ giả.
        sandbox_api_url: format!("{}/sandbox", server.url()),
    };
    SepayEInvoice::new(Client::new(accounts, ConnectionStatus, config).unwrap())
}

fn vnd(minor: i64) -> Money {
    Money::new(minor, Currency::VND)
}

fn invoice(id: Uuid, kind: EInvoiceKind) -> EInvoice {
    let line = EInvoiceLine::new(1, "Sản phẩm A", "2", vnd(100_000), vnd(200_000), vnd(20_000))
        .unit(Some("cái".to_owned()))
        .rate_percent(Some("10".to_owned()));
    let buyer = Party::new("Công ty ABC")
        .tax_code(Some("0101234567".to_owned()))
        .address(Some("123 Đường A, Hà Nội".to_owned()))
        .email(Some("buyer@example.com".to_owned()));
    EInvoice::new(
        id,
        kind,
        Party::new("Cửa hàng"),
        buyer,
        Date::new(2026, 11, 21).unwrap(),
        Currency::VND,
    )
    .series("1C26TAA", Some(7))
    .lines(
        vec![line],
        vec![EInvoiceTaxTotal::new(Some("10".to_owned()), vnd(200_000), vnd(20_000))],
    )
}

fn token_reply(token: &str) -> Value {
    json!({"success": true, "data": {"access_token": token, "token_type": "Bearer", "expires_in": 86400}})
}

fn account_detail() -> Value {
    json!({"success": true, "data": {"id": ACCOUNT, "provider": "matbao", "active": true,
        "templates": [
            {"template_code": "1", "invoice_series": "C26TSE", "invoice_label": "1 - C26TSE - Hóa đơn giá trị gia tăng"},
            {"template_code": "2", "invoice_series": "C26MSE", "invoice_label": "2 - C26MSE - Hóa đơn bán hàng"}]}})
}

fn tracking(code: &str) -> Value {
    json!({"success": true, "data": {"tracking_code": code,
        "tracking_url": format!("https://einvoice-api.sepay.vn/v1/invoices/create/check/{code}"),
        "message": "Đã tạo yêu cầu xuất bán hóa đơn điện tử"}})
}

/// `Authorization: Basic base64("id:secret")`.
fn basic(id: &str, secret: &str) -> String {
    use base64::Engine as _;
    format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{id}:{secret}")))
}

#[tokio::test]
async fn xuat_hoa_don_dung_tai_lieu_bang_tai_khoan_cua_tung_khach() {
    TestDatabase::run(async |test| {
        let server = FakeProviderServer::start().await.unwrap();
        let app = app(test, &server).await;
        for (tenant, id, secret) in [("one", "id-one", "secret-one"), ("two", "id-two", "secret-two")] {
            set(&app, tenant, "client_id", id).await;
            set(&app, tenant, "client_secret", secret).await;
            set(&app, tenant, "provider_account_id", ACCOUNT).await;
        }
        let provider = provider(&app, &server);
        let (one, two) = (app.tenant("one").await.unwrap(), app.tenant("two").await.unwrap());
        server.reply("POST", "/v1/token", 200, token_reply("token-one"));
        server.reply("POST", "/v1/token", 200, token_reply("token-two"));
        server.reply("GET", &format!("/v1/provider-accounts/{ACCOUNT}"), 200, account_detail());
        server.reply("POST", "/v1/invoices/create", 200, tracking("trk-1"));

        let document = Uuid::now_v7();
        let issued = provider.issue(one.id(), &invoice(document, EInvoiceKind::Vat)).await.unwrap();
        assert_eq!(issued.provider_ref, format!("{document}|trk-1"));
        let seen = server.take_requests();
        assert_eq!(seen.len(), 3);
        // Token lấy bằng Basic Authentication của khách một, thân rỗng.
        assert_eq!((seen[0].path.as_str(), seen[0].header("authorization")), ("/v1/token", Some(basic("id-one", "secret-one").as_str())));
        assert!(seen[0].body.is_empty());
        assert!(seen[1..].iter().all(|request| request.header("authorization") == Some("Bearer token-one")));
        let body = seen[2].json();
        assert_eq!(body, json!({
            "provider_account_id": ACCOUNT,
            "reference_code": document.to_string(),
            "template_code": "1",
            // Ký hiệu lấy từ SePay, không phải ký hiệu nội bộ.
            "invoice_series": "C26TSE",
            "issued_date": "2026-11-21 00:00:00",
            "currency": "VND",
            "payment_method": "TM/CK",
            "is_draft": false,
            "buyer": {"type": "company", "name": "Công ty ABC", "tax_code": "0101234567",
                      "address": "123 Đường A, Hà Nội", "email": "buyer@example.com"},
            "items": [{"line_number": 1, "line_type": 1, "item_name": "Sản phẩm A", "unit": "cái",
                       "quantity": 2, "unit_price": 100_000, "tax_rate": 10, "tax_amount": 20_000}],
            "total_amount": 220_000
        }));

        // Hoá đơn bán hàng: mẫu 2, không gửi thuế. Token của khách một được dùng lại (không xin lại).
        let sales = provider.issue(one.id(), &invoice(Uuid::now_v7(), EInvoiceKind::Sales)).await.unwrap();
        assert!(sales.provider_ref.ends_with("|trk-1"));
        let seen = server.take_requests();
        assert!(seen.iter().all(|request| request.path != "/v1/token"));
        let body = seen.last().unwrap().json();
        assert_eq!((&body["template_code"], &body["invoice_series"]), (&json!("2"), &json!("C26MSE")));
        assert!(body["items"][0].get("tax_rate").is_none() && body["items"][0].get("tax_amount").is_none());

        // Khách hai dùng tài khoản thử: mọi lời gọi của khách hai sang địa chỉ sandbox, khách một không.
        set(&app, "two", "environment", "sandbox").await;
        server.reply("POST", "/sandbox/v1/token", 200, token_reply("token-two"));
        server.reply("GET", &format!("/sandbox/v1/provider-accounts/{ACCOUNT}"), 200, account_detail());
        server.reply("POST", "/sandbox/v1/invoices/create", 200, tracking("trk-s"));
        // Khách hai: token riêng, lấy bằng tài khoản riêng.
        provider.issue(two.id(), &invoice(Uuid::now_v7(), EInvoiceKind::Vat)).await.unwrap();
        let seen = server.take_requests();
        assert_eq!(seen[0].header("authorization"), Some(basic("id-two", "secret-two").as_str()));
        assert!(seen[1..].iter().all(|request| request.header("authorization") == Some("Bearer token-two")));
        assert!(seen.iter().all(|request| request.path.starts_with("/sandbox/v1/")), "{seen:?}");
        // Giá trị lạ bị từ chối lúc đặt.
        let uri = "/api/v1/kernel/settings/einvoice_sepay.connection.api.environment";
        let refused = app.put(uri).token("two").json(json!({"value": "test"})).send().await.unwrap();
        assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn gui_lai_trang_thai_va_tai_file() {
    TestDatabase::run(async |test| {
        let server = FakeProviderServer::start().await.unwrap();
        let app = app(test, &server).await;
        set(&app, "one", "client_id", "id-one").await;
        set(&app, "one", "client_secret", "secret-one").await;
        set(&app, "one", "provider_account_id", ACCOUNT).await;
        let provider = provider(&app, &server);
        let one = app.tenant("one").await.unwrap();
        server.reply("POST", "/v1/token", 200, token_reply("token-one"));
        server.reply("GET", &format!("/v1/provider-accounts/{ACCOUNT}"), 200, account_detail());

        // Lần gửi trước đã tới nơi nhưng câu trả lời bị mất: SePay báo mã tham chiếu đã có hoá đơn.
        let document = Uuid::now_v7();
        server.reply(
            "POST",
            "/v1/invoices/create",
            409,
            json!({"success": false, "error": {"code": "EINVOICE_DOCUMENT_EXISTED",
                   "message": "Hệ thống đã ghi nhận hoá đơn cho mã tham chiếu này."}}),
        );
        let again = provider.issue(one.id(), &invoice(document, EInvoiceKind::Vat)).await.unwrap();
        assert_eq!(again.provider_ref, format!("{document}|"));

        // Trạng thái theo mã tracking: đang xử lý → thất bại → thành công (đã có mã cơ quan thuế).
        let check = "/v1/invoices/create/check/trk-1";
        let reference = format!("{document}|trk-1");
        server.reply("GET", check, 200, json!({"success": true, "data": {"status": "Pending", "message": "Đang xử lý",
            "retry": {"retries_count": 1, "max_retries": 7, "next_retry_at": "2026-11-21 10:00:00"}}}));
        server.reply("GET", check, 200, json!({"success": true, "data": {"status": "Failed", "message": "Ký số thất bại"}}));
        server.reply("GET", check, 200, json!({"success": true, "data": {"status": "Success", "message": "Xuất hóa đơn điện tử thành công",
            "invoice": {"reference_code": document.to_string(), "invoice_number": "589", "status": "issued"}}}));
        assert_eq!(provider.status(one.id(), &reference).await.unwrap(), EInvoiceStatus::Pending);
        assert_eq!(
            provider.status(one.id(), &reference).await.unwrap(),
            EInvoiceStatus::Rejected { reason: "Ký số thất bại".to_owned() }
        );
        let detail = format!("/v1/invoices/{document}");
        server.reply("GET", &detail, 200, json!({"success": true, "data": {"reference_code": document.to_string(),
            "invoice_number": "589", "status": "issued", "provider_status": "5"}}));
        server.reply("GET", &detail, 200, json!({"success": true, "data": {"reference_code": document.to_string(),
            "invoice_number": "589", "status": "issued", "provider_status": "4", "tax_authority_code": "M1-26-ABCDE-00000000589"}}));
        // Đã xuất nhưng cơ quan thuế chưa phản hồi: còn chờ.
        assert_eq!(provider.status(one.id(), &reference).await.unwrap(), EInvoiceStatus::Pending);
        let accepted = EInvoiceStatus::Accepted {
            authority_code: Some("M1-26-ABCDE-00000000589".to_owned()),
            number: Some(589),
        };
        assert_eq!(provider.status(one.id(), &reference).await.unwrap(), accepted);
        // Không có mã tracking (trường hợp 409 ở trên): hỏi thẳng theo mã tham chiếu.
        server.take_requests();
        assert_eq!(provider.status(one.id(), &again.provider_ref).await.unwrap(), accepted);
        assert_eq!(server.take_requests().len(), 1);

        // Tải file: nội dung base64 trong `data.content`.
        server.reply("GET", "/v1/invoices/trk-1/download", 200,
            json!({"success": true, "data": {"file_type": "pdf", "file_name": "HD_0000589.pdf", "content": "JVBERi0xLjQK"}}));
        let file = provider.download(one.id(), &reference, EInvoiceFormat::Pdf).await.unwrap();
        assert!(file.starts_with(b"%PDF-1.4"));
        assert_eq!(server.take_requests()[0].query_param("type"), Some("pdf"));

        // Token hết hạn giữa chừng (401): xin token mới và gọi lại một lần.
        server.reply("GET", "/v1/invoices/trk-2/download", 401,
            json!({"success": false, "error": {"code": "TOKEN_EXPIRED", "message": "Token hết hạn"}}));
        server.reply("GET", "/v1/invoices/trk-2/download", 200,
            json!({"success": true, "data": {"file_type": "xml", "file_name": "HD.xml", "content": "PGE+PC9hPg=="}}));
        let xml = provider.download(one.id(), &format!("{document}|trk-2"), EInvoiceFormat::Xml).await.unwrap();
        assert_eq!(&xml[..], b"<a></a>");
        let paths: Vec<String> = server.take_requests().into_iter().map(|request| request.path).collect();
        assert_eq!(paths, ["/v1/invoices/trk-2/download", "/v1/token", "/v1/invoices/trk-2/download"]);

        // Huỷ: tài liệu không có API huỷ hoá đơn đã phát hành.
        let cancel = provider.cancel(one.id(), &EInvoiceCancel::new(reference, "sai")).await.unwrap_err();
        assert_eq!(cancel.code(), "einvoice.not_supported");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn thay_the_dieu_chinh_va_loi() {
    TestDatabase::run(async |test| {
        let server = FakeProviderServer::start().await.unwrap();
        let app = app(test, &server).await;
        set(&app, "one", "client_id", "id-one").await;
        set(&app, "one", "client_secret", "secret-one").await;
        let provider = provider(&app, &server);
        let (one, two) = (app.tenant("one").await.unwrap(), app.tenant("two").await.unwrap());
        server.reply("POST", "/v1/token", 200, token_reply("token-one"));
        // Khách chưa chọn tài khoản nhà cung cấp: tự lấy khi có đúng một tài khoản đang hoạt động.
        server.reply("GET", "/v1/provider-accounts", 200, json!({"data": {"items": [
            {"id": ACCOUNT, "provider": "matbao", "active": true},
            {"id": "other", "provider": "matbao", "active": false}]}}));
        server.reply("POST", "/v1/invoices/replace", 200, tracking("trk-r"));
        server.reply("POST", "/v1/invoices/adjust", 200, tracking("trk-a"));

        let original = Uuid::now_v7();
        let related = |relation| {
            RelatedInvoice::new(relation, "C26TSE", 589, Date::new(2026, 11, 20).unwrap())
                .provider_ref(Some(format!("{original}|trk-1")))
        };
        let replacement = invoice(Uuid::now_v7(), EInvoiceKind::Vat).related(Some(related(Relation::Replacement)));
        let issued = provider.issue(one.id(), &replacement).await.unwrap();
        assert!(issued.provider_ref.ends_with("|trk-r"));
        let body = server.take_requests().last().unwrap().json();
        assert_eq!(body["original_invoice_reference_code"], original.to_string());
        assert_eq!(body["provider_account_id"], ACCOUNT);
        // Thay thế dùng mẫu và ký hiệu của hoá đơn gốc.
        assert!(body.get("template_code").is_none() && body.get("adjustment_type").is_none());

        let mut decrease = invoice(Uuid::now_v7(), EInvoiceKind::Vat).related(Some(related(Relation::Adjustment)));
        decrease.lines = vec![EInvoiceLine::new(1, "Giảm giá", "1", vnd(-50_000), vnd(-50_000), vnd(-5_000))
            .rate_percent(Some("10.00".to_owned()))];
        decrease.totals = vec![EInvoiceTaxTotal::new(Some("10".to_owned()), vnd(-50_000), vnd(-5_000))];
        provider.issue(one.id(), &decrease).await.unwrap();
        let body = server.take_requests().last().unwrap().json();
        assert_eq!((&body["adjustment_type"], &body["total_amount"]), (&json!("decrease"), &json!(-55_000)));
        assert_eq!(body["items"][0]["tax_rate"], 10);

        // Hoá đơn gốc không do bộ nối này gửi; tiền không phải đồng; thuế suất SePay không nhận.
        let mut unknown = invoice(Uuid::now_v7(), EInvoiceKind::Vat);
        unknown.related = Some(RelatedInvoice::new(Relation::Replacement, "C26TSE", 1, Date::new(2026, 11, 20).unwrap()));
        assert_eq!(provider.issue(one.id(), &unknown).await.unwrap_err().code(), "einvoice.rejected");
        let mut dollars = invoice(Uuid::now_v7(), EInvoiceKind::Vat);
        dollars.currency = Currency::USD;
        assert_eq!(provider.issue(one.id(), &dollars).await.unwrap_err().code(), "einvoice.not_supported");
        let mut odd = invoice(Uuid::now_v7(), EInvoiceKind::Vat);
        odd.lines[0].rate_percent = Some("3.5".to_owned());
        assert_eq!(provider.issue(one.id(), &odd).await.unwrap_err().code(), "einvoice.rejected");
        let certificate = invoice(Uuid::now_v7(), EInvoiceKind::WithholdingCertificate);
        assert_eq!(provider.issue(one.id(), &certificate).await.unwrap_err().code(), "einvoice.not_supported");
        // Những hoá đơn trên bị từ chối trước khi gọi ra ngoài.
        assert!(server.take_requests().is_empty());
        server.reply("GET", &format!("/v1/provider-accounts/{ACCOUNT}"), 200, account_detail());

        // Lỗi của SePay: mã lỗi trạng thái tài khoản (500 kèm mã) không phải "thử lại sau"; 500 không
        // mã là lỗi hệ thống.
        server.take_requests();
        server.reply("POST", "/v1/invoices/create", 500,
            json!({"success": false, "error": {"code": "QUOTA_HAS_BEEN_USERD_UP", "message": "Hết hạn ngạch"}}));
        server.reply("POST", "/v1/invoices/create", 500, json!({"message": "Internal Server Error"}));
        let quota = provider.issue(one.id(), &invoice(Uuid::now_v7(), EInvoiceKind::Vat)).await.unwrap_err();
        assert_eq!(quota.code(), "einvoice.rejected");
        assert!(quota.message().contains("QUOTA_HAS_BEEN_USERD_UP"), "{}", quota.message());
        let down = provider.issue(one.id(), &invoice(Uuid::now_v7(), EInvoiceKind::Vat)).await.unwrap_err();
        assert_eq!(down.code(), "einvoice.provider_unavailable");

        // Khách hai chưa cấu hình: lỗi rõ, không dùng tài khoản của khách một, không gọi ra ngoài.
        server.take_requests();
        let missing = provider.issue(two.id(), &invoice(Uuid::now_v7(), EInvoiceKind::Vat)).await.unwrap_err();
        assert_eq!(missing.code(), "provider.not_configured");
        assert!(server.take_requests().is_empty());

        // Thử kết nối qua API cài đặt; client secret không bao giờ được trả lại.
        let test_uri = "/api/v1/kernel/settings/einvoice_sepay.connection.api.client_secret/test";
        let answer = app.request(Method::POST, test_uri).token("one").send().await.unwrap().json();
        assert_eq!(answer["data"]["ok"], true, "{answer}");
        let listed = app.get("/api/v1/kernel/settings?group=connection").token("one").send().await.unwrap().text();
        assert!(!listed.contains("secret-one"), "{listed}");
    })
    .await
    .unwrap();
}
