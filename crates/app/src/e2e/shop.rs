//! Luồng bán hàng mẫu chạy trọn (bước 70 P8, D-20261121-shop-first-order) trên chương trình thật, cả
//! SQLite và Postgres: module thử nghiệm bật bằng `AUTOXIDE_MODULES` như bản cài, dữ liệu tham chiếu
//! nạp bằng task như quickstart, tài khoản nhà cung cấp do khách tự đặt qua API cài đặt. Sản phẩm →
//! giỏ vãng lai → báo phí GHN (người nhận và kích thước kiện thật) → đặt hàng trả bằng MoMo → IPN →
//! đơn xác nhận, tin SMS qua eSMS (trước khi khách đặt tài khoản eSMS: tin bỏ qua "kênh chưa bật") →
//! vận đơn GHN tự đặt → GHN báo đã giao → hoá đơn điện tử qua SePay eInvoice. Mọi nhà cung cấp là
//! `FakeProviderServer` trả lời đúng tài liệu. Từng lệnh khớp hướng dẫn `docs/guides/shop.md`.

use autoxide_kernel::provider::{hex, hmac_sha256};
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgTree, root_of};
use autoxide_kernel::time::Timestamp;
use autoxide_testkit::{FakeProviderServer, TestDatabase};
use axum::http::StatusCode;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::e2e::media::{admin_token, call, start};
use crate::runtime::Runtime;

/// Như dòng `.env` của người mở cửa hàng: module chính thức cộng bốn bộ nối.
const MODULES: &str = "official,momo,ghn,esms,einvoice_sepay";
const PUBLIC: &str = "https://shop.example";
const PHONE: &str = "0912345678";

// Đường dẫn của nhà cung cấp, theo tài liệu công khai của họ.
const MOMO_CREATE: &str = "/v2/gateway/api/create";
const GHN_PREVIEW: &str = "/shiip/public-api/v2/shipping-order/preview";
const GHN_CREATE: &str = "/shiip/public-api/v2/shipping-order/create";
const ESMS_SEND: &str = "/MainService.svc/json/SendMultipleMessage_V4_post_json/";

/// Tài khoản MoMo của cửa hàng: (Partner Code, Access Key, Secret Key).
const MOMO: (&str, &str, &str) = ("MOMOSHOP", "access-of-shop", "secret-of-shop");
const GHN_SHOP_ID: i64 = 885_123;
const SEPAY_ACCOUNT: &str = "0aea3134-da40-11f0-aef4-52c7e9b4f41b";

struct Providers {
    momo: FakeProviderServer,
    ghn: FakeProviderServer,
    esms: FakeProviderServer,
    einvoice: FakeProviderServer,
}

struct Shop {
    runtime: Runtime,
    bearer: String,
}

impl Shop {
    /// Gọi API như người quản trị cửa hàng (`Authorization: Bearer …`).
    async fn admin(&self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let headers =
            [("authorization", self.bearer.as_str()), ("content-type", "application/json")];
        let body = body.map(|body| body.to_string().into_bytes()).unwrap_or_default();
        let (status, _, answer) = call(&self.runtime, method, uri, &headers, body).await;
        (status, answer)
    }

    /// Gọi API không đăng nhập (người mua vãng lai, webhook của nhà cung cấp).
    async fn public(
        &self,
        method: &str,
        uri: &str,
        headers: &[(&str, &str)],
        body: &Value,
    ) -> (StatusCode, Value) {
        let mut all = vec![("content-type", "application/json")];
        all.extend_from_slice(headers);
        let (status, _, answer) =
            call(&self.runtime, method, uri, &all, body.to_string().into_bytes()).await;
        (status, answer)
    }

    async fn set(&self, name: &str, value: &str) {
        let uri = format!("/api/v1/kernel/settings/{name}");
        let (status, body) = self.admin("PUT", &uri, Some(json!({"value": value}))).await;
        assert_eq!(status, StatusCode::OK, "{name}: {body}");
    }

    /// Chạy hết event và job đang chờ, như tiến trình nền của `serve`.
    async fn drain(&self) {
        let worker =
            self.runtime.modules.worker(4).unwrap().blocked_delay(std::time::Duration::ZERO);
        let dispatcher = self.runtime.modules.dispatcher().unwrap();
        for _ in 0..12 {
            let events = dispatcher.run_once().await.unwrap();
            if worker.run_once().await.unwrap() + events == 0 {
                break;
            }
        }
    }

    async fn messages(&self) -> Vec<Value> {
        let (status, page) =
            self.admin("GET", "/api/v1/notifications/messages?limit=100", None).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        page["data"].as_array().unwrap().clone()
    }
}

/// Bản cài như quickstart, cộng các dòng `.env` của hướng dẫn; dữ liệu tham chiếu nạp bằng task.
async fn open(test: &TestDatabase, providers: &Providers) -> Shop {
    let dir = std::env::temp_dir().join(format!("autoxide-shop-{}", Uuid::now_v7()));
    let pairs = [
        ("AUTOXIDE_FILES_DIR", dir.display().to_string()),
        ("AUTOXIDE_MODULES", MODULES.to_owned()),
        ("AUTOXIDE_HTTP_PUBLIC_URL", PUBLIC.to_owned()),
        // Chỉ test mới đổi địa chỉ của nhà cung cấp (mặc định là địa chỉ thật của họ).
        ("AUTOXIDE_MOMO_API_URL", providers.momo.url().to_owned()),
        ("AUTOXIDE_GHN_API_URL", providers.ghn.url().to_owned()),
        ("AUTOXIDE_ESMS_API_URL", providers.esms.url().to_owned()),
        ("AUTOXIDE_EINVOICE_SEPAY_API_URL", providers.einvoice.url().to_owned()),
    ];
    let runtime = start(test.url(), &pairs).await;
    let tasks = runtime.modules.tasks().unwrap();
    for name in ["address.import", "tax.import_pack"] {
        let (_, task) = tasks.iter().find(|(full, _)| full == name).unwrap();
        task.run(&task.parse(&[]).unwrap()).await.unwrap();
    }
    let write = runtime.database.begin_write().await.unwrap();
    // `organizations` có thể đã tạo nút gốc.
    let _ = OrgTree::insert_root(&write, DEFAULT_TENANT).await;
    write.commit().await.unwrap();
    let token = admin_token(&runtime, "owner@example.com").await;
    Shop { runtime, bearer: format!("Bearer {token}") }
}

fn ghn_ok(data: Value) -> Value {
    json!({"code": 200, "message": "Success", "data": data})
}

fn vnd(amount: i64) -> Value {
    json!({"amount": amount.to_string(), "currency": "VND"})
}

/// IPN của MoMo cho thanh toán `order_id`, ký đúng tài liệu bằng Secret Key của cửa hàng.
fn momo_ipn(order_id: &str, amount: i64) -> Value {
    let (message, order_info, order_type, pay_type, trans_id) =
        ("Thành công.", "", "momo_wallet", "qr", 4_100_000_001_i64);
    let response_time = 1_795_000_100_000_i64;
    let raw = format!(
        "accessKey={}&amount={amount}&extraData=&message={message}&orderId={order_id}\
         &orderInfo={order_info}&orderType={order_type}&partnerCode={}&payType={pay_type}\
         &requestId={order_id}&responseTime={response_time}&resultCode=0&transId={trans_id}",
        MOMO.1, MOMO.0
    );
    json!({
        "partnerCode": MOMO.0, "orderId": order_id, "requestId": order_id, "amount": amount,
        "orderInfo": order_info, "orderType": order_type, "transId": trans_id, "resultCode": 0,
        "message": message, "payType": pay_type, "responseTime": response_time, "extraData": "",
        "signature": hex(&hmac_sha256(MOMO.2.as_bytes(), raw.as_bytes())),
    })
}

/// Callback trạng thái của GHN (tài liệu "Order Status Callback").
fn ghn_callback(order_code: &str, status: &str, minutes: i64) -> Value {
    let time = Timestamp::from_unix_seconds(Timestamp::now().as_unix_seconds() + minutes * 60)
        .unwrap()
        .to_rfc3339();
    json!({
        "ShopID": GHN_SHOP_ID, "Time": time, "OrderCode": order_code, "ClientOrderCode": "",
        "Type": "switch_status", "Description": "Cập nhật trạng thái", "Status": status,
        "Reason": "", "ReasonCode": "", "CODAmount": 0, "CODTransferDate": null, "Weight": 1200,
        "ConvertedWeight": 1200, "Length": 30, "Width": 20, "Height": 20, "PaymentType": 1,
        "IsPartialReturn": false, "PartialReturnCode": "", "Fee": {}, "TotalFee": 0,
        "Warehouse": "", "ShipperName": "", "ShipperPhone": "", "PodURL": "",
    })
}

#[tokio::test]
async fn mo_cua_hang_ban_don_dau_tien_tu_dau_den_cuoi() {
    TestDatabase::run(async |test| {
        let providers = Providers {
            momo: FakeProviderServer::start().await.unwrap(),
            ghn: FakeProviderServer::start().await.unwrap(),
            esms: FakeProviderServer::start().await.unwrap(),
            einvoice: FakeProviderServer::start().await.unwrap(),
        };
        let shop = open(test, &providers).await;
        let org = root_of(DEFAULT_TENANT).as_uuid();

        // ---- 1. Tài khoản nhà cung cấp của chính cửa hàng ----------------------------------------
        for (field, value) in [("partner_code", MOMO.0), ("access_key", MOMO.1), ("secret_key", MOMO.2)] {
            shop.set(&format!("momo.connection.account.{field}"), value).await;
        }
        for (field, value) in
            [("client_id", "einvoice-id"), ("client_secret", "einvoice-secret"), ("provider_account_id", SEPAY_ACCOUNT)]
        {
            shop.set(&format!("einvoice_sepay.connection.api.{field}"), value).await;
        }
        let ghn_account = json!({"org_id": org, "carrier_code": "ghn", "name": "GHN",
            "credentials": {"token": "ghn-token"}, "settings": {"shop_id": GHN_SHOP_ID}});
        let (status, account) =
            shop.admin("POST", "/api/v1/shipping/carrier-accounts", Some(ghn_account)).await;
        assert_eq!(status, StatusCode::CREATED, "{account}");
        let ghn_webhook = account["data"]["webhook_path"].as_str().unwrap().to_owned();
        let method = json!({"org_id": org, "code": "ghn", "kind": "carrier",
            "names": {"vi": "Giao Hàng Nhanh", "en": "GHN"}, "currency": "VND",
            "carrier_account_id": account["data"]["id"], "auto_book": true});
        let (status, created) = shop.admin("POST", "/api/v1/shipping/methods", Some(method)).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let year = i32::from(Timestamp::now().date_utc().year());
        let series = json!({"kind": "vat_invoice", "symbol": "1C26TAA", "year": year, "einvoice_provider": "sepay"});
        let (status, created) = shop.admin("POST", "/api/v1/invoices/series", Some(series)).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");

        // ---- 2. Sản phẩm, giá, tồn kho -------------------------------------------------------------
        let product = json!({"kind": "physical", "tax_category": "vat.standard", "locale": "vi",
            "text": {"name": "Ấm trà"}, "variant": {"sku": "TEA-1", "weight_grams": 600,
            "length_mm": 300, "width_mm": 200, "height_mm": 100}});
        let (status, created) = shop.admin("POST", "/api/v1/catalog/products", Some(product)).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let product_id = created["data"]["id"].as_str().unwrap().to_owned();
        let variant = created["data"]["variants"][0]["id"].as_str().unwrap().to_owned();
        let list = json!({"name": "Bán lẻ", "currency": "VND", "tax_inclusive": true});
        let (status, list) = shop.admin("POST", "/api/v1/catalog/price-lists", Some(list)).await;
        assert_eq!(status, StatusCode::CREATED, "{list}");
        let prices = json!({"reference": "opening", "prices": [{"variant_id": variant, "amount": vnd(150_000)}]});
        let uri = format!("/api/v1/catalog/price-lists/{}/prices", list["data"]["id"].as_str().unwrap());
        let (status, added) = shop.admin("POST", &uri, Some(prices)).await;
        assert_eq!(status, StatusCode::OK, "{added}");
        let stock = json!({"org_id": org, "reference": "opening-1", "lines": [{"variant_id": variant,
            "quantity": 10, "reason_code": "found", "unit_cost": vnd(90_000)}]});
        let (status, made) = shop.admin("POST", "/api/v1/inventory/adjustments", Some(stock)).await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        let headers =
            [("authorization", shop.bearer.as_str()), ("content-type", "application/json"), ("if-match", "*")];
        let uri = format!("/api/v1/catalog/products/{product_id}/status");
        let (status, _, active) =
            call(&shop.runtime, "POST", &uri, &headers, json!({"status": "active"}).to_string().into_bytes()).await;
        assert_eq!(status, StatusCode::OK, "{active}");

        // ---- 3. Người mua vãng lai: giỏ, địa chỉ, phí GHN ------------------------------------------
        providers.ghn.reply("POST", GHN_PREVIEW, 200, ghn_ok(json!({"order_code": "",
            "total_fee": 32_000, "expected_delivery_time": "2026-12-01T16:59:59Z"})));
        let (status, cart) = shop.public("POST", "/api/v1/orders/carts", &[], &json!({})).await;
        assert_eq!(status, StatusCode::CREATED, "{cart}");
        let cart_id = cart["data"]["id"].as_str().unwrap().to_owned();
        let cart_token = cart["data"]["cart_token"].as_str().unwrap().to_owned();
        let guest = [("cart-token", cart_token.as_str())];
        let uri = format!("/api/v1/orders/carts/{cart_id}/lines/{variant}");
        let (status, cart) = shop.public("PUT", &uri, &guest, &json!({"quantity": 2})).await;
        assert_eq!(status, StatusCode::OK, "{cart}");
        let mut delivery = json!({"recipient_name": "Nguyễn An", "phone": PHONE,
            "address": {"country": "VN", "province_code": "01", "commune_code": "00004", "line": "12 Tràng Tiền"}});
        let uri = format!("/api/v1/orders/carts/{cart_id}/delivery");
        let (status, cart) = shop.public("PUT", &uri, &guest, &delivery).await;
        assert_eq!(status, StatusCode::OK, "{cart}");
        let option = cart["data"]["shipping"]["options"]
            .as_array()
            .unwrap()
            .iter()
            .find(|option| option["amount"]["amount"] == "32000")
            .unwrap_or_else(|| panic!("{cart}"))
            .clone();
        delivery["shipping_option"] = option["option_key"].clone();
        let (status, cart) = shop.public("PUT", &uri, &guest, &delivery).await;
        assert_eq!(status, StatusCode::OK, "{cart}");
        assert_eq!(cart["data"]["totals"]["total"]["amount"], "332000", "{cart}");
        // GHN nhận người nhận thật và kích thước của kiện (không còn giá trị giữ chỗ).
        let preview = providers.ghn.take_requests().into_iter().rev()
            .find(|request| request.path == GHN_PREVIEW)
            .unwrap()
            .json();
        assert_eq!((&preview["to_name"], &preview["to_phone"]), (&json!("Nguyễn An"), &json!(PHONE)));
        assert_eq!((&preview["length"], &preview["width"], &preview["height"]), (&json!(30), &json!(20), &json!(20)));

        // ---- 4. Đặt hàng, trả bằng MoMo ------------------------------------------------------------
        providers.momo.reply("POST", MOMO_CREATE, 200, json!({"partnerCode": MOMO.0, "resultCode": 0,
            "message": "Thành công.", "payUrl": "https://test-payment.momo.vn/pay/abc",
            "deeplink": "momo://app?action=pay", "qrCodeUrl": "00020101021226110007vn.momo",
            "responseTime": 1_795_000_000_000_i64}));
        let checkout = json!({"cart_id": cart_id, "quote_token": cart["data"]["quote_token"],
            "payment_method": "wallet", "contact": {"name": "Nguyễn An", "phone": PHONE},
            "return_url": "https://shop.example/thank-you"});
        let headers = [guest[0], ("idempotency-key", "first-order")];
        let (status, placed) = shop.public("POST", "/api/v1/orders/checkout", &headers, &checkout).await;
        assert_eq!(status, StatusCode::CREATED, "{placed}");
        let order_id = placed["data"]["orders"][0]["id"].as_str().unwrap().to_owned();
        assert_eq!(placed["data"]["orders"][0]["status"], "pending_payment");
        let action = &placed["data"]["payment_actions"][0];
        assert_eq!(action["kind"], "redirect", "{placed}");
        assert_eq!(action["redirect_url"], "https://test-payment.momo.vn/pay/abc", "{placed}");
        let sent = providers.momo.take_requests().into_iter().find(|r| r.path == MOMO_CREATE).unwrap().json();
        assert_eq!((&sent["partnerCode"], &sent["amount"]), (&json!(MOMO.0), &json!(332_000)));
        let payment_id = sent["orderId"].as_str().unwrap().to_owned();

        // Thư "đã nhận đơn" đi bằng SMS; cửa hàng chưa đặt tài khoản eSMS: bỏ qua, kênh chưa bật.
        shop.drain().await;
        let placed_sms = shop.messages().await.into_iter()
            .find(|row| row["kind"] == "orders.order_placed" && row["channel"] == "sms")
            .unwrap();
        assert_eq!((&placed_sms["status"], &placed_sms["skip_reason"]), (&json!("skipped"), &json!("channel_disabled")), "{placed_sms}");
        assert!(providers.esms.take_requests().is_empty());

        // ---- 5. Cửa hàng đặt eSMS; MoMo báo đã thu → đơn xác nhận, SMS đi ----------------------------
        for (field, value) in [("api_key", "esms-key"), ("secret_key", "esms-secret"), ("brandname", "AMTRA")] {
            shop.set(&format!("esms.connection.sms.{field}"), value).await;
        }
        providers.esms.reply("POST", ESMS_SEND, 200, json!({"CodeResult": "100", "CountRegenerate": 0, "SMSID": "sms-1"}));
        let (status, hooks) = shop.admin("GET", "/api/v1/kernel/connections/webhooks", None).await;
        assert_eq!(status, StatusCode::OK, "{hooks}");
        let momo_hook = hooks["data"].as_array().unwrap().iter()
            .find(|row| row["connection"] == "momo.connection.account")
            .unwrap()["path"].as_str().unwrap().to_owned();
        let (status, answer) = shop.public("POST", &momo_hook, &[], &momo_ipn(&payment_id, 332_000)).await;
        assert!(status.is_success(), "{status} {answer}");
        shop.drain().await;
        let (_, order) = shop.admin("GET", &format!("/api/v1/orders/{order_id}"), None).await;
        assert_eq!((&order["data"]["status"], &order["data"]["payment_status"]), (&json!("confirmed"), &json!("paid")), "{order}");
        let sms = providers.esms.take_requests();
        let texts: Vec<Value> = sms.iter().map(|request| request.json()).collect();
        assert!(!texts.is_empty(), "no SMS sent");
        assert!(texts.iter().all(|body| body["Phone"] == PHONE && body["Brandname"] == "AMTRA"), "{texts:?}");

        // ---- 6. Giao hàng qua GHN ------------------------------------------------------------------
        providers.ghn.reply("POST", GHN_CREATE, 200, ghn_ok(json!({"order_code": "GHNTEA1",
            "fee": {"main_service": 32000}, "total_fee": 32000, "expected_delivery_time": "2026-12-01T16:59:59Z"})));
        let uri = format!("/api/v1/orders/{order_id}/shipments");
        let (status, requested) = shop.admin("POST", &uri, Some(json!({}))).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{requested}");
        shop.drain().await;
        let booked = providers.ghn.take_requests().into_iter().find(|r| r.path == GHN_CREATE).expect("GHN order created").json();
        assert_eq!((&booked["to_name"], &booked["to_phone"]), (&json!("Nguyễn An"), &json!(PHONE)));
        assert_eq!(booked["to_ward_name"], "Phường Ba Đình");

        // ---- 7. GHN báo đã lấy, đã giao → hoá đơn điện tử qua SePay ---------------------------------
        let einvoice = &providers.einvoice;
        einvoice.reply("POST", "/v1/token", 200, json!({"success": true, "data": {"access_token": "sepay-token", "token_type": "Bearer", "expires_in": 86400}}));
        einvoice.reply("GET", &format!("/v1/provider-accounts/{SEPAY_ACCOUNT}"), 200, json!({"success": true, "data": {"id": SEPAY_ACCOUNT, "active": true,
            "templates": [{"template_code": "1", "invoice_series": "C26TSE", "invoice_label": "1 - C26TSE"}]}}));
        einvoice.reply("POST", "/v1/invoices/create", 200, json!({"success": true, "data": {"tracking_code": "trk-1",
            "tracking_url": "https://einvoice-api.sepay.vn/v1/invoices/create/check/trk-1"}}));
        einvoice.reply("GET", "/v1/invoices/create/check/trk-1", 200, json!({"success": true, "data": {"status": "Pending", "message": "Đang xử lý"}}));
        for (status, minutes) in [("picked", 1), ("delivered", 2)] {
            let (code, answer) = shop.public("POST", &ghn_webhook, &[], &ghn_callback("GHNTEA1", status, minutes)).await;
            assert!(code.is_success(), "{code} {answer}");
            shop.drain().await;
        }
        let (_, order) = shop.admin("GET", &format!("/api/v1/orders/{order_id}"), None).await;
        assert_eq!(order["data"]["status"], "delivered", "{order}");
        let created = einvoice.take_requests().into_iter().find(|r| r.path == "/v1/invoices/create").expect("e-invoice sent").json();
        assert_eq!(created["provider_account_id"], SEPAY_ACCOUNT);
        assert_eq!(created["currency"], "VND");
        assert_eq!(created["is_draft"], false);
        let uri = format!("/api/v1/invoices/documents?source_module=orders&source_id={order_id}");
        let (status, invoices) = shop.admin("GET", &uri, None).await;
        assert_eq!(status, StatusCode::OK, "{invoices}");
        let document = &invoices["data"][0];
        assert_eq!(document["kind"], "vat_invoice", "{invoices}");
        // Gửi rồi, SePay còn xử lý (hỏi lại bằng job hay `…/refresh`).
        assert!(["submitting", "accepted"].contains(&document["status"].as_str().unwrap()), "{document}");
        assert_eq!(created["reference_code"], document["id"]);
        // Tin "đã giao" cũng đi qua eSMS.
        assert!(!providers.esms.take_requests().is_empty());
    })
    .await
    .unwrap();
}
