//! Phần còn lại của bán hàng dùng được như thật (cả SQLite và Postgres): đơn trả bằng thẻ qua cổng (giả)
//! với mã giảm giá — gửi lại link thanh toán, trả lại suất khuyến mại và hàng đang giữ bằng tay; hàng số
//! (file của biến thể, thu hồi quyền tải của đơn); bán tại quầy (ca, tra mã vạch); địa chỉ đã lưu của
//! người mua; chứng từ đọc bằng tool.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::payment::PaymentGateway;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, root_of};
use autoxide_testkit::{FakePaymentGateway, FakeProviderServer, TestApp, TestDatabase};
use serde_json::{Value, json};
use uuid::Uuid;

use super::kit::{
    ADMIN, ANONYMOUS, app_with, call_tool, drain, expect, get, id, import_reference_data,
    promotion_window,
};

const BUYER: &str = "buyer";

fn vnd(amount: i64) -> Value {
    json!({"amount": amount.to_string(), "currency": "VND"})
}

async fn activate_product(app: &TestApp, product: &str) {
    let answer = app
        .post(&format!("/api/v1/catalog/products/{product}/status"))
        .token(ADMIN)
        .header("if-match", "*")
        .json(json!({"status": "active"}))
        .send()
        .await
        .unwrap();
    assert_eq!(answer.status(), 200, "{}", answer.text());
}

/// Cửa hàng thu qua cổng giả, giao tiêu chuẩn 30.000đ; trả (app, cổng, biến thể ấm trà, ID bảng giá).
async fn shop(
    test: &TestDatabase,
    settings: &[(&str, &str)],
) -> (TestApp, Arc<FakePaymentGateway>, String, String) {
    let gateway = Arc::new(FakePaymentGateway::new());
    let contributed = gateway.clone();
    let app = app_with(test, move |mut builder| {
        for (name, value) in settings {
            builder = builder.setting(name, value);
        }
        builder
            .contribute::<dyn PaymentGateway>(contributed)
            .principal(BUYER, Principal::customer(Uuid::from_u128(0xb2)))
    })
    .await;
    import_reference_data(&app).await;
    let setting = json!({"value": "fake"});
    expect(
        &app,
        "PUT",
        "/api/v1/kernel/settings/ledger.payments.default_provider",
        ADMIN,
        Some(setting),
        200,
    )
    .await;
    // Trang quay về sau khi trả phải thuộc cửa hàng (P-5, D-20261125-return-url-allowlist).
    let origins = json!({"value": "https://shop.example"});
    let path = "/api/v1/kernel/settings/ledger.payments.return_origins";
    expect(&app, "PUT", path, ADMIN, Some(origins), 200).await;
    let root = json!(root_of(DEFAULT_TENANT));
    let zone = json!({"org_id": root, "name": "Toàn quốc", "regions": [{"country": "VN"}]});
    let zone = id(&expect(&app, "POST", "/api/v1/shipping/zones", ADMIN, Some(zone), 201).await);
    let method = json!({"org_id": root, "code": "standard", "kind": "table_rate", "currency": "VND",
                        "names": {"vi": "Giao tiêu chuẩn", "en": "Standard"}});
    let method =
        id(&expect(&app, "POST", "/api/v1/shipping/methods", ADMIN, Some(method), 201).await);
    let rates = json!({"rates": [{"zone_id": zone, "min_weight_grams": 0, "amount": vnd(30_000)}]});
    expect(
        &app,
        "PUT",
        &format!("/api/v1/shipping/methods/{method}/rates"),
        ADMIN,
        Some(rates),
        200,
    )
    .await;
    let product = json!({"kind": "physical", "tax_category": "vat.standard", "locale": "vi",
        "text": {"name": "Ấm trà"}, "variant": {"sku": "TEA-1", "barcode": "8930000000019", "weight_grams": 600}});
    let created = expect(&app, "POST", "/api/v1/catalog/products", ADMIN, Some(product), 201).await;
    let product = id(&created);
    let variant = created["data"]["variants"][0]["id"].as_str().unwrap().to_owned();
    let list = json!({"name": "Bán lẻ", "currency": "VND", "tax_inclusive": true});
    let list =
        id(&expect(&app, "POST", "/api/v1/catalog/price-lists", ADMIN, Some(list), 201).await);
    let prices = json!({"reference": "opening", "prices": [{"variant_id": variant, "amount": vnd(150_000)}]});
    expect(
        &app,
        "POST",
        &format!("/api/v1/catalog/price-lists/{list}/prices"),
        ADMIN,
        Some(prices),
        200,
    )
    .await;
    let stock = json!({"org_id": root, "reference": "opening-1", "lines": [{"variant_id": variant,
        "quantity": 20, "reason_code": "found", "unit_cost": vnd(90_000)}]});
    expect(&app, "POST", "/api/v1/inventory/adjustments", ADMIN, Some(stock), 201).await;
    activate_product(&app, &product).await;
    (app, gateway, variant, list)
}

#[tokio::test]
async fn don_the_ma_giam_gia_link_thanh_toan_va_tra_lai_suat_giu() {
    TestDatabase::run(async |test| {
        let (app, _gateway, variant, _) = shop(test, &[]).await;
        let root = json!(root_of(DEFAULT_TENANT));
        let (starts, ends) = promotion_window();
        let promotion = json!({"org_id": root, "kind": "percent_off", "currency": "VND", "value_bp": 1000,
            "trigger": "code", "locale": "vi", "title": "Giảm 10%", "terms": "Nhập mã", "starts_at": starts, "ends_at": ends});
        let promotion = call_tool(&app, ADMIN, "promotions.create_promotion", json!({"promotion": promotion})).await;
        let promotion = promotion["id"].as_str().unwrap().to_owned();
        call_tool(&app, ADMIN, "promotions.generate_codes", json!({"id": promotion, "purpose": "redeem", "code": "GIAM10"})).await;
        super::orders::activate(&app, &promotion).await;

        // Khách vãng lai: giỏ, mã giảm giá, giao hàng, trả bằng thẻ.
        let cart = expect(&app, "POST", "/api/v1/orders/carts", ANONYMOUS, Some(json!({})), 201).await;
        let (cart, token) = (id(&cart), cart["data"]["cart_token"].as_str().unwrap().to_owned());
        let guest = |method: &str, path: &str, body: Value| {
            let method = axum::http::Method::from_bytes(method.as_bytes()).unwrap();
            app.request(method, path).header("cart-token", &token).json(body)
        };
        let path = format!("/api/v1/orders/carts/{cart}");
        assert_eq!(guest("PUT", &format!("{path}/lines/{variant}"), json!({"quantity": 1})).send().await.unwrap().status(), 200);
        assert_eq!(guest("PUT", &format!("{path}/codes"), json!({"codes": ["GIAM10"]})).send().await.unwrap().status(), 200);
        let mut delivery = json!({"recipient_name": "An", "phone": "0901234567",
            "address": {"country": "VN", "province_code": "01", "commune_code": "00004", "line": "12 Tràng Tiền"}});
        let priced = guest("PUT", &format!("{path}/delivery"), delivery.clone()).send().await.unwrap().json();
        delivery["shipping_option"] = priced["data"]["shipping"]["options"][0]["option_key"].clone();
        let priced = guest("PUT", &format!("{path}/delivery"), delivery).send().await.unwrap().json();
        let body = json!({"cart_id": cart, "quote_token": priced["data"]["quote_token"], "payment_method": "card",
                          "return_url": "https://shop.example/done", "contact": {"name": "An", "phone": "0901234567"}});
        let answer = guest("POST", "/api/v1/orders/checkout", body).header("idempotency-key", "card-1").send().await.unwrap();
        let placed = answer.json();
        assert_eq!(answer.status(), 201, "{placed}");
        let order = placed["data"]["orders"][0]["id"].as_str().unwrap().to_owned();

        // Người mua đóng trang thanh toán: nhân viên gửi lại link.
        expect(&app, "POST", &format!("/api/v1/orders/{order}/payment-link"), ADMIN, Some(json!({})), 200).await;

        // Đơn kẹt: trả suất khuyến mại và hàng đang giữ bằng tay.
        let held = get(&app, "/api/v1/promotions/redemptions").await;
        let redemption = held["data"][0]["id"].as_str().unwrap_or_else(|| panic!("{held}")).to_owned();
        expect(&app, "POST", &format!("/api/v1/promotions/redemptions/{redemption}/release"), ADMIN, None, 200).await;
        let reservations = get(&app, "/api/v1/inventory/reservations").await;
        let reservation = reservations["data"][0]["id"].as_str().unwrap_or_else(|| panic!("{reservations}")).to_owned();
        call_tool(&app, ADMIN, "inventory.release_reservation", json!({"id": reservation})).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hang_so_quay_ban_hang_dia_chi_da_luu_va_chung_tu() {
    TestDatabase::run(async |test| {
        let (app, gateway, _variant, list) = shop(test, &[]).await;
        let root = json!(root_of(DEFAULT_TENANT));

        // Hàng số: file của biến thể, gỡ một file.
        let product = json!({"kind": "digital", "tax_category": "vat.standard", "locale": "vi",
            "text": {"name": "Khoá học"}, "variant": {"sku": "COURSE-1"}});
        let created = expect(&app, "POST", "/api/v1/catalog/products", ADMIN, Some(product), 201).await;
        let variant = created["data"]["variants"][0]["id"].as_str().unwrap().to_owned();
        let mut files = Vec::new();
        for name in ["bai-1.pdf", "bai-2.pdf"] {
            let uploaded = app
                .post(&format!("/api/v1/files?name={name}"))
                .token(ADMIN)
                .header("content-type", "application/pdf")
                .bytes(b"%PDF-1.4 course".to_vec())
                .send()
                .await
                .unwrap();
            assert_eq!(uploaded.status(), 201, "{}", uploaded.text());
            let file = json!({"file_id": uploaded.json()["data"]["id"]});
            let asset = expect(&app, "POST", &format!("/api/v1/catalog/variants/{variant}/digital-assets"), ADMIN, Some(file), 201).await;
            files.push(id(&asset));
        }
        get(&app, &format!("/api/v1/catalog/variants/{variant}/digital-assets")).await;
        expect(&app, "DELETE", &format!("/api/v1/catalog/digital-assets/{}", files[1]), ADMIN, None, 204).await;
        let prices = json!({"reference": "course", "prices": [{"variant_id": variant, "amount": vnd(500_000)}]});
        expect(&app, "POST", &format!("/api/v1/catalog/price-lists/{list}/prices"), ADMIN, Some(prices), 200).await;
        activate_product(&app, created["data"]["id"].as_str().unwrap()).await;

        // Người mua (đã đăng nhập) mua khoá học, trả thẻ; đối tác báo đã thu → quyền tải; cửa hàng thu hồi.
        let cart = id(&expect(&app, "POST", "/api/v1/orders/carts", BUYER, Some(json!({})), 201).await);
        let path = format!("/api/v1/orders/carts/{cart}/lines/{variant}");
        let priced = expect(&app, "PUT", &path, BUYER, Some(json!({"quantity": 1})), 200).await;
        let body = json!({"cart_id": cart, "quote_token": priced["data"]["quote_token"], "payment_method": "card",
                          "return_url": "https://shop.example/done", "contact": {"name": "An", "phone": "0901234567"}});
        let answer = app.post("/api/v1/orders/checkout").token(BUYER).header("idempotency-key", "course-1").json(body).send().await.unwrap();
        let placed = answer.json();
        assert_eq!(answer.status(), 201, "{placed}");
        let order = placed["data"]["orders"][0]["id"].as_str().unwrap().to_owned();
        let payment = *gateway.payments.lock().unwrap().keys().next().unwrap();
        let money = autoxide_kernel::money::Money::new(500_000, autoxide_kernel::money::Currency::VND);
        let (headers, body) = gateway.paid(payment, money, "evt-course-1");
        let mut webhook = app.post("/api/v1/ledger/webhooks/fake").bytes(body).header("content-type", "application/json");
        for (name, value) in &headers {
            webhook = webhook.header(name, value);
        }
        assert!(webhook.send().await.unwrap().status().is_success());
        drain(&app).await;
        expect(&app, "POST", &format!("/api/v1/catalog/orders/{order}/downloads/revoke"), ADMIN, None, 200).await;

        // Bán tại quầy: mở ca, tra mã vạch.
        let shift = json!({"org_id": root, "register_code": "Q1", "opening_cash": vnd(1_000_000)});
        let shift = id(&expect(&app, "POST", "/api/v1/orders/pos/shifts", ADMIN, Some(shift), 201).await);
        get(&app, &format!("/api/v1/orders/pos/catalog?shift_id={shift}&barcode=8930000000019")).await;

        // Người mua lưu địa chỉ giao hàng rồi xem lại.
        let address = json!({"recipient_name": "An", "phone": "0901234567", "label": "Nhà",
            "address": {"country": "VN", "province_code": "01", "commune_code": "00004", "line": "12 Tràng Tiền"}});
        let saved = id(&expect(&app, "POST", "/api/v1/shipping/me/addresses", BUYER, Some(address), 201).await);
        expect(&app, "GET", &format!("/api/v1/shipping/me/addresses/{saved}"), BUYER, None, 200).await;

        // Chứng từ đọc bằng tool.
        let year = i32::from(autoxide_kernel::time::Timestamp::now().date_utc().year());
        let series = json!({"kind": "sales_invoice", "symbol": "2C26TAA", "year": year});
        expect(&app, "POST", "/api/v1/invoices/series", ADMIN, Some(series), 201).await;
        let document = json!({"kind": "sales_invoice", "currency": "VND", "reference": "manual-1",
            "buyer": {"kind": "consumer", "name": "An"},
            "lines": [{"description": "Tư vấn", "quantity": "1", "unit_price": vnd(200_000), "tax_category": "vat.standard"}]});
        let document = id(&expect(&app, "POST", "/api/v1/invoices/documents", ADMIN, Some(document), 201).await);
        call_tool(&app, ADMIN, "invoices.document", json!({"document_id": document})).await;
        drain(&app).await;
    })
    .await
    .unwrap();
}

/// Đường gửi tin của eSMS (tài liệu công khai của eSMS).
const ESMS_SEND: &str = "/MainService.svc/json/SendMultipleMessage_V4_post_json/";

#[tokio::test]
async fn khach_vang_lai_tra_don_bang_ma_sms_roi_xem_va_tai_hoa_don() {
    TestDatabase::run(async |test| {
        let esms = FakeProviderServer::start().await.unwrap();
        let (app, _gateway, variant, _) = shop(test, &[("AUTOXIDE_ESMS_API_URL", esms.url())]).await;
        // Cửa hàng bật SMS (eSMS, máy chủ giả) và có dãy số hoá đơn: giao xong thì tự xuất hoá đơn.
        for (field, value) in [("api_key", "esms-key"), ("secret_key", "esms-secret"), ("brandname", "AMTRA")] {
            let path = format!("/api/v1/kernel/settings/esms.connection.sms.{field}");
            expect(&app, "PUT", &path, ADMIN, Some(json!({"value": value})), 200).await;
        }
        esms.reply("POST", ESMS_SEND, 200, json!({"CodeResult": "100", "CountRegenerate": 0, "SMSID": "sms-1"}));
        let year = i32::from(autoxide_kernel::time::Timestamp::now().date_utc().year());
        let series = json!({"kind": "vat_invoice", "symbol": "1C26TAA", "year": year});
        expect(&app, "POST", "/api/v1/invoices/series", ADMIN, Some(series), 201).await;

        // Khách vãng lai đặt đơn COD.
        let cart = expect(&app, "POST", "/api/v1/orders/carts", ANONYMOUS, Some(json!({})), 201).await;
        let (cart, token) = (id(&cart), cart["data"]["cart_token"].as_str().unwrap().to_owned());
        let guest = |method: &str, path: &str, body: Value| {
            let method = axum::http::Method::from_bytes(method.as_bytes()).unwrap();
            app.request(method, path).header("cart-token", &token).json(body)
        };
        let path = format!("/api/v1/orders/carts/{cart}");
        assert_eq!(guest("PUT", &format!("{path}/lines/{variant}"), json!({"quantity": 1})).send().await.unwrap().status(), 200);
        let mut delivery = json!({"recipient_name": "An", "phone": "0901234567",
            "address": {"country": "VN", "province_code": "01", "commune_code": "00004", "line": "12 Tràng Tiền"}});
        let priced = guest("PUT", &format!("{path}/delivery"), delivery.clone()).send().await.unwrap().json();
        delivery["shipping_option"] = priced["data"]["shipping"]["options"][0]["option_key"].clone();
        let priced = guest("PUT", &format!("{path}/delivery"), delivery).send().await.unwrap().json();
        let body = json!({"cart_id": cart, "quote_token": priced["data"]["quote_token"], "payment_method": "cod",
                          "contact": {"name": "An", "phone": "0901234567"}});
        let answer = guest("POST", "/api/v1/orders/checkout", body).header("idempotency-key", "guest-cod-1").send().await.unwrap();
        let placed = answer.json();
        assert_eq!(answer.status(), 201, "{placed}");
        let order = placed["data"]["orders"][0]["id"].as_str().unwrap().to_owned();
        let number = placed["data"]["orders"][0]["number"].clone();
        for to in ["preparing", "shipping", "delivered"] {
            call_tool(&app, ADMIN, "orders.transition_order", json!({"id": order, "to": to})).await;
        }
        drain(&app).await;

        // Khách tra đơn bằng số đơn và số điện thoại, nhận mã qua SMS, xác nhận lấy token xem đơn.
        esms.take_requests();
        let lookup = json!({"number": number, "phone": "0901234567"});
        let lookup = expect(&app, "POST", "/api/v1/orders/public/orders/lookup", ANONYMOUS, Some(lookup), 200).await;
        drain(&app).await;
        let texts: Vec<String> = esms
            .take_requests()
            .iter()
            .filter_map(|request| request.json()["Content"].as_str().map(str::to_owned))
            .collect();
        let code = texts
            .iter()
            // Tin có cả số đơn (`2026-000001`): giữ dấu `-` để không nhầm phần đuôi của số đơn là mã.
            .flat_map(|text| text.split(|c: char| !c.is_ascii_digit() && c != '-'))
            .find(|word| word.len() == 6 && word.bytes().all(|b| b.is_ascii_digit()))
            .unwrap_or_else(|| panic!("no code in {texts:?}"))
            .to_owned();
        let confirm = format!("/api/v1/orders/public/orders/lookup/{}/confirm", lookup["data"]["lookup_id"].as_str().unwrap());
        let access = expect(&app, "POST", &confirm, ANONYMOUS, Some(json!({"code": code})), 200).await;
        let access = access["data"]["access_token"].as_str().unwrap().to_owned();

        // Xem chứng từ của đơn và tải hoá đơn PDF.
        let answer = app
            .get(&format!("/api/v1/orders/public/orders/{order}/documents"))
            .header("order-token", &access)
            .send()
            .await
            .unwrap();
        let documents = answer.json();
        assert_eq!(answer.status(), 200, "{documents}");
        let document = documents["data"][0]["document_id"].as_str().unwrap_or_else(|| panic!("{documents}")).to_owned();
        let answer = app
            .get(&format!("/api/v1/orders/public/orders/{order}/documents/{document}"))
            .header("order-token", &access)
            .send()
            .await
            .unwrap();
        assert_eq!(answer.status(), 200, "{}", answer.text());
        assert_eq!(answer.header("content-type"), "application/pdf");
    })
    .await
    .unwrap();
}

/// Đặt một đơn trả thẻ (một ấm trà, giao tiêu chuẩn) với trang quay về `return_url`; `buyer`: token của người mua
/// đã đăng nhập, `None`: khách vãng lai (giỏ theo `cart-token`). Trả thân câu trả lời của checkout (201).
async fn card_checkout(
    app: &TestApp,
    variant: &str,
    buyer: Option<&str>,
    return_url: &str,
    key: &str,
) -> Value {
    let who = buyer.unwrap_or(ANONYMOUS);
    let cart = expect(app, "POST", "/api/v1/orders/carts", who, Some(json!({})), 201).await;
    let cart_token = cart["data"]["cart_token"].as_str().map(str::to_owned);
    let cart = id(&cart);
    let send = |method: &str, path: &str, body: Value| {
        let method = axum::http::Method::from_bytes(method.as_bytes()).unwrap();
        let request = app.request(method, path).json(body);
        match (buyer, &cart_token) {
            (Some(token), _) => request.token(token),
            (None, Some(cart_token)) => request.header("cart-token", cart_token),
            (None, None) => panic!("a guest cart has a cart_token"),
        }
    };
    let path = format!("/api/v1/orders/carts/{cart}");
    assert_eq!(
        send("PUT", &format!("{path}/lines/{variant}"), json!({"quantity": 1}))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    let mut delivery = json!({"recipient_name": "An", "phone": "0901234567",
        "address": {"country": "VN", "province_code": "01", "commune_code": "00004", "line": "12 Tràng Tiền"}});
    let priced =
        send("PUT", &format!("{path}/delivery"), delivery.clone()).send().await.unwrap().json();
    delivery["shipping_option"] = priced["data"]["shipping"]["options"][0]["option_key"].clone();
    let priced = send("PUT", &format!("{path}/delivery"), delivery).send().await.unwrap().json();
    let body = json!({"cart_id": cart, "quote_token": priced["data"]["quote_token"], "payment_method": "card",
                      "return_url": return_url, "contact": {"name": "An", "phone": "0901234567"}});
    let answer = send("POST", "/api/v1/orders/checkout", body)
        .header("idempotency-key", key)
        .send()
        .await
        .unwrap();
    let placed = answer.json();
    assert_eq!(answer.status(), 201, "{placed}");
    placed
}

/// P-5 (pentest 2026-11): `return_url` không thuộc cửa hàng bị từ chối — qua checkout (đơn vẫn đặt, lý do ở
/// `payment_actions[0].error` là `orders.return_url_not_allowed`, không bị gộp thành "phương thức không dùng được")
/// và qua link thanh toán của khách vãng lai lẫn người mua đã đăng nhập; địa chỉ của cửa hàng thì mở được.
#[tokio::test]
async fn return_url_la_bi_tu_choi_qua_checkout_va_link_thanh_toan() {
    TestDatabase::run(async |test| {
        let esms = FakeProviderServer::start().await.unwrap();
        let (app, _gateway, variant, _) =
            shop(test, &[("AUTOXIDE_ESMS_API_URL", esms.url())]).await;
        for (field, value) in
            [("api_key", "esms-key"), ("secret_key", "esms-secret"), ("brandname", "AMTRA")]
        {
            let path = format!("/api/v1/kernel/settings/esms.connection.sms.{field}");
            expect(&app, "PUT", &path, ADMIN, Some(json!({"value": value})), 200).await;
        }
        esms.reply(
            "POST",
            ESMS_SEND,
            200,
            json!({"CodeResult": "100", "CountRegenerate": 0, "SMSID": "sms-1"}),
        );
        let refused = |action: &Value| {
            assert_eq!(action["kind"], "none", "{action}");
            assert_eq!(action["error"]["code"], "orders.return_url_not_allowed", "{action}");
        };

        // Khách vãng lai: checkout với trang lạ → đơn đặt, chưa có lượt thu, lý do rõ.
        let placed =
            card_checkout(&app, &variant, None, "https://evil.example/done", "guest-evil").await;
        refused(&placed["data"]["payment_actions"][0]);
        let order = placed["data"]["orders"][0]["id"].as_str().unwrap().to_owned();
        let number = placed["data"]["orders"][0]["number"].clone();
        drain(&app).await;
        esms.take_requests();
        let lookup = json!({"number": number, "phone": "0901234567"});
        let lookup = expect(
            &app,
            "POST",
            "/api/v1/orders/public/orders/lookup",
            ANONYMOUS,
            Some(lookup),
            200,
        )
        .await;
        drain(&app).await;
        let texts: Vec<String> = esms
            .take_requests()
            .iter()
            .filter_map(|request| request.json()["Content"].as_str().map(str::to_owned))
            .collect();
        let code = texts
            .iter()
            .flat_map(|text| text.split(|c: char| !c.is_ascii_digit() && c != '-'))
            .find(|word| word.len() == 6 && word.bytes().all(|b| b.is_ascii_digit()))
            .unwrap_or_else(|| panic!("no code in {texts:?}"))
            .to_owned();
        let confirm = format!(
            "/api/v1/orders/public/orders/lookup/{}/confirm",
            lookup["data"]["lookup_id"].as_str().unwrap()
        );
        let access =
            expect(&app, "POST", &confirm, ANONYMOUS, Some(json!({"code": code})), 200).await;
        let access = access["data"]["access_token"].as_str().unwrap().to_owned();
        let guest_link = |url: &str, key: &str| {
            app.post(&format!("/api/v1/orders/public/orders/{order}/payment-link"))
                .header("order-token", &access)
                .header("idempotency-key", key)
                .json(json!({"return_url": url}))
                .send()
        };
        let answer = guest_link("https://shop.example.evil.example/done", "g1").await.unwrap();
        assert_eq!(answer.status(), 422, "{}", answer.text());
        assert_eq!(answer.json()["code"], "orders.return_url_not_allowed");
        let answer = guest_link("https://shop.example/done", "g2").await.unwrap();
        assert_eq!(answer.status(), 200, "{}", answer.text());
        assert_eq!(answer.json()["data"][0]["kind"], "redirect");

        // Người mua đã đăng nhập: cùng như vậy qua `me/orders/{id}/payment-link`.
        let placed = card_checkout(
            &app,
            &variant,
            Some(BUYER),
            "http://shop.example:8080/done",
            "buyer-evil",
        )
        .await;
        refused(&placed["data"]["payment_actions"][0]);
        let order = placed["data"]["orders"][0]["id"].as_str().unwrap().to_owned();
        let path = format!("/api/v1/orders/me/orders/{order}/payment-link");
        let link = |url: &str, key: &str| {
            app.post(&path)
                .token(BUYER)
                .header("idempotency-key", key)
                .json(json!({"return_url": url}))
                .send()
        };
        let answer = link("https://evil.example/done", "b1").await.unwrap();
        assert_eq!(answer.status(), 422, "{}", answer.text());
        assert_eq!(answer.json()["code"], "orders.return_url_not_allowed");
        // Sai định dạng (không phải http(s)) là lý do khác, câu khác.
        for (url, key) in [("ftp://shop.example/done", "b3"), ("//shop.example/done", "b4")] {
            let answer = link(url, key).await.unwrap();
            assert_eq!(answer.status(), 422, "{}", answer.text());
            assert_eq!(answer.json()["code"], "orders.return_url_invalid", "{url}");
        }
        let answer = link("https://shop.example/thanks", "b2").await.unwrap();
        assert_eq!(answer.status(), 200, "{}", answer.text());
        assert_eq!(answer.json()["data"][0]["kind"], "redirect");
    })
    .await
    .unwrap();
}
