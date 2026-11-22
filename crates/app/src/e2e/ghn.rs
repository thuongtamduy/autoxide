//! Test của bộ nối GHN cùng module `shipping` thật (test của một module không dựng module khác —
//! gate `module-import`), trên database gate đang chọn (SQLite và Postgres), với máy chủ giả dựng
//! theo đúng tài liệu của GHN (`FakeProviderServer`): request đúng tài liệu, hai khách hai tài
//! khoản không lẫn nhau, khách cấu hình thiếu báo lỗi rõ, lỗi của GHN thành `code` ổn định, webhook
//! tìm đúng khách, gửi lại không xử lý trùng.

use std::sync::Arc;

use autoxide_address::api::AdministrativeUnits;
use autoxide_kernel::tenancy::{OrgTree, root_of};
use autoxide_kernel::time::Timestamp;
use autoxide_testkit::{FakeProviderServer, TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use uuid::Uuid;

const STAFF: [&str; 6] = [
    "shipping.settings.manage",
    "shipping.read",
    "shipping.carriers.manage",
    "shipping.shipments.read",
    "shipping.shipments.manage",
    "shipping.cod.manage",
];

const SHOPS: &str = "/shiip/public-api/v2/shop/all";
const PREVIEW: &str = "/shiip/public-api/v2/shipping-order/preview";
const CREATE: &str = "/shiip/public-api/v2/shipping-order/create";
const DETAIL: &str = "/shiip/public-api/v2/shipping-order/detail";
const BY_CLIENT_CODE: &str = "/shiip/public-api/v2/shipping-order/detail-by-client-code";
const CANCEL: &str = "/shiip/public-api/v2/switch-status/cancel";
const PRINT_TOKEN: &str = "/shiip/public-api/v2/a5/gen-token";

/// App có `shipping` + `ghn`, hai khách `one` và `two` (mỗi khách một quản lý), GHN trỏ về máy chủ
/// giả: tài khoản thường về `server`, tài khoản `sandbox` về `staging`.
async fn app(test: &TestDatabase) -> (TestApp, FakeProviderServer, FakeProviderServer) {
    let server = FakeProviderServer::start().await.unwrap();
    let staging = FakeProviderServer::start().await.unwrap();
    let app = TestApp::builder(test)
        .setting("AUTOXIDE_GHN_API_URL", server.url())
        .setting("AUTOXIDE_GHN_SANDBOX_API_URL", staging.url())
        .tenant("one")
        .tenant("two")
        .user_in("one", "boss-one", 1, &STAFF)
        .user_in("two", "boss-two", 2, &STAFF)
        .provide::<dyn AdministrativeUnits>(Arc::new(
            autoxide_address::api::testing::FakeAdministrativeUnits::new()
                .province("79", "Thành phố Hồ Chí Minh")
                .commune("26734", "Phường Sài Gòn", "79"),
        ))
        .module(autoxide_shipping::module())
        .module(autoxide_ghn::module())
        .build()
        .await
        .unwrap();
    for slug in ["one", "two"] {
        let tenant = app.tenant(slug).await.unwrap();
        let write = tenant.database().begin_write().await.unwrap();
        OrgTree::insert_root(&write, tenant.id()).await.unwrap();
        write.commit().await.unwrap();
    }
    (app, server, staging)
}

async fn org_of(app: &TestApp, slug: &str) -> Uuid {
    root_of(app.tenant(slug).await.unwrap().id()).as_uuid()
}

async fn call(
    app: &TestApp,
    method: &str,
    uri: &str,
    who: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method, &format!("/api/v1/shipping{uri}")).token(who);
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let status = answer.status();
    (status, if answer.body().is_empty() { Value::Null } else { answer.json() })
}

fn vnd(amount: i64) -> Value {
    json!({"amount": amount.to_string(), "currency": "VND"})
}

// Lưu ý về `FakeProviderServer`: câu trả lời của một đường dẫn xếp hàng, câu cuối dùng lại mãi —
// nên bài test xếp MỌI câu của một đường dẫn theo đúng thứ tự trước lời gọi đầu tiên tới nó.

fn ok(data: Value) -> Value {
    json!({"code": 200, "message": "Success", "data": data})
}

/// Tài khoản GHN của khách `slug`; trả (ID tài khoản, `webhook_path`).
async fn account(
    app: &TestApp,
    slug: &str,
    who: &str,
    token: &str,
    shop_id: i64,
) -> (String, String) {
    let body = json!({
        "org_id": org_of(app, slug).await, "carrier_code": "ghn", "name": "GHN",
        "credentials": {"token": token}, "settings": {"shop_id": shop_id},
    });
    let (status, created) = call(app, "POST", "/carrier-accounts", who, Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert!(!created.to_string().contains(token), "the token never comes back");
    (
        created["data"]["id"].as_str().unwrap().to_owned(),
        created["data"]["webhook_path"].as_str().unwrap().to_owned(),
    )
}

/// Phương thức giao bằng tài khoản hãng `account_id`.
async fn method(app: &TestApp, slug: &str, who: &str, account_id: &str) -> String {
    let body = json!({
        "org_id": org_of(app, slug).await, "code": "ghn", "kind": "carrier",
        "names": {"vi": "Giao Hàng Nhanh", "en": "GHN"}, "currency": "VND",
        "carrier_account_id": account_id, "auto_book": false,
    });
    let (status, created) = call(app, "POST", "/methods", who, Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    created["data"]["id"].as_str().unwrap().to_owned()
}

/// Vận đơn 800 g, thu hộ 285 000 đ, tới Phường Sài Gòn.
async fn shipment(app: &TestApp, slug: &str, who: &str, method: &str) -> String {
    let body = json!({
        "order_id": Uuid::now_v7(), "order_number": "DH0001", "request_key": Uuid::now_v7(),
        "org_id": org_of(app, slug).await, "method_id": method,
        "recipient_name": "Tran Minh Anh", "recipient_phone": "0987654321",
        "address": {"country": "VN", "province_code": "79", "commune_code": "26734", "line": "72 Lê Thánh Tôn"},
        "items": [{"order_line_key": "l1", "variant_id": Uuid::now_v7(), "sku": "A", "name": "Ao", "quantity": 2, "weight_grams": 400}],
        "declared_value": vnd(7_000_000), "cod_amount": vnd(285_000), "note": "Gọi trước khi giao",
    });
    let (status, created) = call(app, "POST", "/shipments", who, Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    created["data"]["id"].as_str().unwrap().to_owned()
}

fn created_order(code: &str) -> Value {
    ok(json!({"order_code": code, "fee": {"main_service": 20900}, "total_fee": 20900,
        "expected_delivery_time": "2026-07-15T16:59:59Z"}))
}

/// Đặt vận đơn với GHN (máy chủ giả đã xếp sẵn câu trả lời mang mã `code`); trả ID vận đơn.
async fn booked(app: &TestApp, slug: &str, who: &str, method: &str, code: &str) -> String {
    let id = shipment(app, slug, who, method).await;
    let (status, body) = call(app, "POST", &format!("/shipments/{id}/book"), who, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["tracking_number"], code);
    id
}

/// Chạy hết event và job đang chờ.
async fn drain(app: &TestApp) {
    let worker = app.modules().worker(4).unwrap().blocked_delay(std::time::Duration::ZERO);
    let dispatcher = app.modules().dispatcher().unwrap();
    for _ in 0..10 {
        let events = dispatcher.run_once().await.unwrap();
        if worker.run_once().await.unwrap() + events == 0 {
            break;
        }
    }
}

/// Thân webhook như tài liệu "Order Status Callback" (đủ trường, khoá PascalCase).
fn callback(shop_id: i64, order: &str, kind: &str, status: &str, time: &str) -> Value {
    json!({
        "ShopID": shop_id, "Time": time, "OrderCode": order, "ClientOrderCode": "",
        "Type": kind, "Description": "Cập nhật trạng thái đơn hàng", "Status": status,
        "Reason": "", "ReasonCode": "", "CODAmount": 285000, "CODTransferDate": null,
        "Weight": 800, "ConvertedWeight": 800, "Length": 10, "Width": 10, "Height": 10,
        "PaymentType": 1, "IsPartialReturn": false, "PartialReturnCode": "", "Fee": {},
        "TotalFee": 0, "Warehouse": "", "ShipperName": "", "ShipperPhone": "", "PodURL": "",
    })
}

/// Thời điểm `minutes` phút sau bây giờ, dạng GHN gửi. (Cập nhật có thời điểm trước lần đổi trạng
/// thái gần nhất được ghi vào hành trình nhưng không đổi trạng thái — đúng thiết kế của `shipping`.)
fn after(minutes: i64) -> String {
    Timestamp::from_unix_seconds(Timestamp::now().as_unix_seconds() + minutes * 60)
        .unwrap()
        .to_rfc3339()
}

/// GHN gọi webhook: không token đăng nhập, không header khách.
async fn deliver(app: &TestApp, path: &str, body: &Value) -> (StatusCode, Value) {
    let answer = app
        .post(path)
        .header("content-type", "application/json")
        .bytes(body.to_string().into_bytes())
        .unchecked()
        .send()
        .await
        .unwrap();
    (answer.status(), if answer.body().is_empty() { Value::Null } else { answer.json() })
}

async fn status_of(app: &TestApp, who: &str, id: &str) -> Value {
    call(app, "GET", &format!("/shipments/{id}"), who, None).await.1["data"].clone()
}

#[tokio::test]
async fn thu_ket_noi_hai_khach_hai_tai_khoan() {
    TestDatabase::run(async |test| {
        let (app, server, _staging) = app(test).await;
        let (one, _) = account(&app, "one", "boss-one", "token-of-one", 111).await;
        let (two, _) = account(&app, "two", "boss-two", "token-of-two", 222).await;
        let shops = ok(json!({"last_offset": 222, "shops": [
            {"_id": 111, "name": "Shop one", "status": 1},
            {"_id": 222, "name": "Shop two", "status": 1},
            {"_id": 333, "name": "Locked", "status": 2},
        ]}));
        // Ba lần thử thấy cửa hàng ở trang đầu; lần tìm cửa hàng 999 đọc tiếp trang hai (rỗng);
        // rồi cửa hàng 333 ở trang đầu; sau đó GHN từ chối token.
        let empty = ok(json!({"last_offset": 222, "shops": []}));
        for page in [&shops, &shops, &shops, &empty, &shops] {
            server.reply("GET", SHOPS, 200, page.clone());
        }
        let denied =
            json!({"code": 401, "message": "Authorization header is required!", "data": null});
        server.reply("GET", SHOPS, 401, denied);

        let test_of = |id: String| format!("/carrier-accounts/{id}/test");
        let (status, tested) = call(&app, "POST", &test_of(one.clone()), "boss-one", None).await;
        assert_eq!(status, StatusCode::OK, "{tested}");
        assert_eq!(tested["data"]["check_status"], "ok", "{tested}");
        let (_, tested) = call(&app, "POST", &test_of(two.clone()), "boss-two", None).await;
        assert_eq!(tested["data"]["check_status"], "ok", "{tested}");
        // Mỗi khách gọi GHN bằng đúng token và ShopId của mình.
        let seen = server.take_requests();
        assert_eq!(seen.len(), 2);
        assert_eq!((seen[0].method.as_str(), seen[0].path.as_str()), ("GET", SHOPS));
        assert_eq!(seen[0].header("token"), Some("token-of-one"));
        assert_eq!(seen[0].header("shopid"), Some("111"));
        assert_eq!(seen[0].query_param("limit"), Some("200"));
        assert_eq!(seen[1].header("token"), Some("token-of-two"));
        assert_eq!(seen[1].header("shopid"), Some("222"));
        // Khách này không thấy, không thử được tài khoản của khách kia.
        let (status, _) = call(&app, "POST", &test_of(two.clone()), "boss-one", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(server.take_requests().is_empty());

        // Cửa hàng không thuộc token, hoặc đang bị khoá.
        for shop_id in [999, 333] {
            let changes = json!({"settings": {"shop_id": shop_id}});
            call(&app, "PATCH", &format!("/carrier-accounts/{one}"), "boss-one", Some(changes))
                .await;
            let (_, tested) = call(&app, "POST", &test_of(one.clone()), "boss-one", None).await;
            assert_eq!(tested["data"]["check_status"], "failed", "{tested}");
            assert_eq!(tested["data"]["check_error_code"], "ghn.shop_not_found");
        }
        // GHN từ chối token.
        let (_, tested) = call(&app, "POST", &test_of(one.clone()), "boss-one", None).await;
        assert_eq!(tested["data"]["check_error_code"], "shipping.carrier_unauthorized");
        // Thiếu ShopId: báo rõ thiếu gì, không gọi GHN.
        server.take_requests();
        let changes = json!({"settings": {}});
        call(&app, "PATCH", &format!("/carrier-accounts/{one}"), "boss-one", Some(changes)).await;
        let (_, tested) = call(&app, "POST", &test_of(one), "boss-one", None).await;
        assert_eq!(tested["data"]["check_error_code"], "ghn.account_incomplete");
        assert!(server.take_requests().is_empty());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn bao_gia_dat_in_huy_dung_tai_lieu() {
    TestDatabase::run(async |test| {
        let (app, server, staging) = app(test).await;
        let (account_id, _) = account(&app, "one", "boss-one", "token-of-one", 111).await;
        let method_id = method(&app, "one", "boss-one", &account_id).await;

        // GHN nhận đơn đầu, từ chối số điện thoại của đơn thứ hai, rồi không trả lời.
        server.reply("POST", CREATE, 200, created_order("LADFYR"));
        server.reply("POST", CREATE, 400, json!({"code": 400, "message": "Sai số điện thoại",
            "data": null, "code_message": "PHONE_INVALID"}));
        server.reply("POST", CREATE, 500, json!({"code": 500, "code_message": "SERVER_ERROR_COMMON"}));

        // Báo giá: `preview`, địa chỉ hai cấp theo tên.
        server.reply("POST", PREVIEW, 200, ok(json!({"order_code": "", "total_fee": 20900,
            "expected_delivery_time": "2026-07-16T16:59:59Z"})));
        server.reply("POST", PREVIEW, 400, json!({"code": 400, "data": null,
            "message": "calculate fee error: calculate service fee error: route not found service"}));
        let ask = json!({"org_id": org_of(&app, "one").await,
            "destination": {"country": "VN", "province_code": "79", "commune_code": "26734", "line": "72 Lê Thánh Tôn"},
            "items": [{"variant_id": Uuid::now_v7(), "quantity": 1, "weight_grams": 600}], "order_value": vnd(285_000)});
        let (status, quote) = call(&app, "POST", "/quote", "boss-one", Some(ask.clone())).await;
        assert_eq!(status, StatusCode::OK, "{quote}");
        let option = &quote["data"]["options"][0];
        assert_eq!(option["carrier_code"], "ghn", "{quote}");
        assert_eq!(option["amount"]["amount"], "20900");
        let seen = server.take_requests();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].header("token"), Some("token-of-one"));
        assert_eq!(seen[0].header("shopid"), Some("111"));
        assert_eq!(seen[0].header("content-type"), Some("application/json"));
        let sent = seen[0].json();
        assert_eq!(sent["to_ward_name"], "Phường Sài Gòn");
        assert_eq!(sent["to_province_name"], "Thành phố Hồ Chí Minh");
        assert_eq!(sent["is_new_to_address"], true);
        assert!(sent.get("to_district_name").is_none(), "the new model has no district");
        assert_eq!(sent["service_type_id"], 2);
        assert_eq!(sent["weight"], 600);
        assert_eq!(sent["insurance_value"], 285_000);
        assert_eq!(sent["required_note"], "KHONGCHOXEMHANG");
        assert_eq!(sent["payment_type_id"], 1);
        // GHN không phục vụ tuyến: phương thức báo không dùng được, báo giá không hỏng.
        let (status, quote) = call(&app, "POST", "/quote", "boss-one", Some(ask)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(quote["data"]["unavailable"][0]["reason_code"], "ghn.route_not_supported", "{quote}");
        server.take_requests();

        // Đặt: `create` với `client_order_code` = ID vận đơn.
        let id = booked(&app, "one", "boss-one", &method_id, "LADFYR").await;
        let seen = server.take_requests();
        assert_eq!(seen.len(), 1);
        assert_eq!((seen[0].method.as_str(), seen[0].path.as_str()), ("POST", CREATE));
        assert_eq!(seen[0].header("token"), Some("token-of-one"));
        let sent = seen[0].json();
        assert_eq!(sent["client_order_code"], id);
        assert_eq!(sent["to_name"], "Tran Minh Anh");
        assert_eq!(sent["to_phone"], "0987654321");
        assert_eq!(sent["to_address"], "72 Lê Thánh Tôn, Phường Sài Gòn, Thành phố Hồ Chí Minh");
        assert_eq!(sent["is_new_to_address"], true);
        assert_eq!(sent["weight"], 800);
        assert_eq!((&sent["length"], &sent["width"], &sent["height"]), (&json!(10), &json!(10), &json!(10)));
        assert_eq!(sent["cod_amount"], 285_000);
        assert_eq!(sent["insurance_value"], 5_000_000, "declared value is capped at the GHN maximum");
        assert_eq!(sent["note"], "Gọi trước khi giao");
        assert!(sent.get("from_address").is_none(), "GHN uses the address of the shop");
        assert_eq!(status_of(&app, "boss-one", &id).await["status"], "ready_to_ship");

        // In: xin token in rồi tải trang in A5.
        server.reply("POST", PRINT_TOKEN, 200, ok(json!({"token": "print-token-1"})));
        server.reply_raw("GET", "/a5/public-api/printA5", 200, "text/html", "<html>LADFYR</html>");
        let label = app
            .get(&format!("/api/v1/shipping/shipments/{id}/label"))
            .token("boss-one")
            .send()
            .await
            .unwrap();
        assert_eq!(label.status(), StatusCode::OK);
        assert_eq!(label.header("content-type"), "text/html; charset=utf-8");
        assert_eq!(label.text(), "<html>LADFYR</html>");
        let seen = server.take_requests();
        assert_eq!(seen[0].json(), json!({"order_codes": ["LADFYR"]}));
        assert_eq!(seen[1].query_param("token"), Some("print-token-1"));
        assert_eq!(seen[1].header("token"), None, "the account token is not sent to the print page");

        // Huỷ: GHN báo không huỷ được nữa → 409; rồi huỷ được.
        server.reply("POST", CANCEL, 200, ok(json!([{"order_code": "LADFYR", "result": false, "message": "x"}])));
        server.reply("POST", CANCEL, 200, ok(json!([{"order_code": "LADFYR", "result": true, "message": "OK"}])));
        let cancel = format!("/shipments/{id}/cancel");
        let (status, problem) = call(&app, "POST", &cancel, "boss-one", Some(json!({}))).await;
        assert_eq!(status, StatusCode::CONFLICT, "{problem}");
        assert_eq!(problem["code"], "shipping.cancel_too_late");
        let (status, body) = call(&app, "POST", &cancel, "boss-one", Some(json!({}))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(server.take_requests()[0].json(), json!({"order_codes": ["LADFYR"]}));
        assert_eq!(status_of(&app, "boss-one", &id).await["status"], "cancelled");

        // GHN từ chối số điện thoại: vận đơn về `pending`, lỗi có mã ổn định.
        let refused = shipment(&app, "one", "boss-one", &method_id).await;
        let (status, problem) =
            call(&app, "POST", &format!("/shipments/{refused}/book"), "boss-one", None).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        assert_eq!(problem["code"], "shipping.booking_rejected");
        let after = status_of(&app, "boss-one", &refused).await;
        assert_eq!(after["status"], "pending");
        assert_eq!(after["last_error_code"], "ghn.phone_invalid");
        // GHN không trả lời: lỗi tạm, vận đơn ở `booking`; lần sau tra theo `client_order_code`
        // trước nên không tạo đơn thứ hai.
        let (status, problem) =
            call(&app, "POST", &format!("/shipments/{refused}/book"), "boss-one", None).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{problem}");
        assert_eq!(problem["code"], "shipping.carrier_unavailable");
        assert!(staging.take_requests().is_empty(), "a live account never calls staging");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tim_lai_don_va_tra_trang_thai() {
    TestDatabase::run(async |test| {
        let (app, server, staging) = app(test).await;
        // Tài khoản `sandbox` gọi môi trường staging của GHN.
        let body = json!({
            "org_id": org_of(&app, "one").await, "carrier_code": "ghn", "name": "GHN staging",
            "credentials": {"token": "staging-token"}, "settings": {"shop_id": "196560", "label_size": "80x80"},
            "sandbox": true,
        });
        let (status, created) = call(&app, "POST", "/carrier-accounts", "boss-one", Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let registry = app.api::<dyn autoxide_shipping::api::CarrierRegistry>().unwrap();
        let carrier = registry.get("ghn").unwrap();
        let account = autoxide_shipping::api::CarrierAccount::new(
            Uuid::now_v7(),
            "ghn",
            true,
            json!({"shop_id": "196560", "label_size": "80x80"}),
            autoxide_shipping::api::CarrierCredentials::new(
                [("token".to_owned(), "staging-token".to_owned())].into(),
            ),
        );

        // Chưa có đơn với mã này.
        staging.reply("GET", BY_CLIENT_CODE, 400, json!({"code": 400, "data": null,
            "message": "Lỗi gọi API: corev2_tenant_order_detail - Đơn hàng không tồn tại"}));
        staging.reply("GET", BY_CLIENT_CODE, 200, ok(json!({"order_code": "LADH94",
            "client_order_code": "ref-1", "status": "picked"})));
        assert_eq!(carrier.find_by_reference(&account, "ref-1").await.unwrap(), None);
        // Đã có.
        let found = carrier.find_by_reference(&account, "ref-1").await.unwrap().unwrap();
        assert_eq!(found.tracking_number, "LADH94");
        assert_eq!(found.status, autoxide_shipping::api::ShipmentStatus::PickedUp);
        let seen = staging.take_requests();
        assert_eq!(seen[1].query_param("client_order_code"), Some("ref-1"));
        assert_eq!(seen[1].header("token"), Some("staging-token"));
        assert_eq!(seen[1].header("shopid"), Some("196560"));

        // Tra trạng thái: một cập nhật từ `detail`.
        staging.reply("GET", DETAIL, 200, ok(json!({"order_code": "LADH94", "status": "delivered",
            "updated_date": "2026-07-15T04:49:47.811Z", "cod_amount": 285000, "is_cod_collected": true})));
        let updates = carrier.track(&account, "LADH94").await.unwrap();
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].status, autoxide_shipping::api::ShipmentStatus::Delivered);
        assert_eq!(updates[0].carrier_status, "delivered");
        assert_eq!(
            updates[0].occurred_at,
            Timestamp::parse_rfc3339("2026-07-15T04:49:47.811Z").unwrap()
        );
        assert_eq!(updates[0].cod_collected.map(|money| money.minor()), Some(285_000));
        assert_eq!(staging.take_requests()[0].query_param("order_code"), Some("LADH94"));

        // Khổ giấy của tài khoản chọn trang in; PDF nhận ra theo đầu file.
        staging.reply("POST", PRINT_TOKEN, 200, ok(json!({"token": "p"})));
        staging.reply_raw("GET", "/a5/public-api/print80x80", 200, "application/pdf", "%PDF-1.7 …");
        let label = carrier.label(&account, "LADH94").await.unwrap();
        assert_eq!(label.content_type, "application/pdf");
        assert!(server.take_requests().is_empty(), "a sandbox account never calls production");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn webhook_tim_dung_khach_va_khong_xu_ly_trung() {
    TestDatabase::run(async |test| {
        let (app, server, _staging) = app(test).await;
        let (account_one, hook_one) = account(&app, "one", "boss-one", "token-of-one", 111).await;
        let (account_two, hook_two) = account(&app, "two", "boss-two", "token-of-two", 222).await;
        let method_one = method(&app, "one", "boss-one", &account_one).await;
        let method_two = method(&app, "two", "boss-two", &account_two).await;
        // Hai khách có hai vận đơn TRÙNG mã ở GHN: không được lẫn.
        server.reply("POST", CREATE, 200, created_order("LADFYR"));
        let one = booked(&app, "one", "boss-one", &method_one, "LADFYR").await;
        let two = booked(&app, "two", "boss-two", &method_two, "LADFYR").await;
        assert!(hook_one.starts_with("/api/v1/shipping/webhooks/ghn/"), "{hook_one}");
        assert_ne!(hook_one, hook_two);

        // GHN gọi địa chỉ của khách `one`, không header nào: đúng khách, đúng vận đơn.
        let picked = callback(111, "LADFYR", "switch_status", "picked", &after(1));
        let (status, receipt) = deliver(&app, &hook_one, &picked).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{receipt}");
        drain(&app).await;
        assert_eq!(status_of(&app, "boss-one", &one).await["status"], "picked_up");
        assert_eq!(status_of(&app, "boss-two", &two).await["status"], "ready_to_ship");

        // GHN gửi lại đúng callback đó (gửi lại khi quá giờ): nhận, nhưng chỉ áp một lần.
        let before = status_of(&app, "boss-one", &one).await;
        let (status, _) = deliver(&app, &hook_one, &picked).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        drain(&app).await;
        assert_eq!(
            status_of(&app, "boss-one", &one).await,
            before,
            "a repeated callback changes nothing"
        );

        // Loại callback không đổi trạng thái: nhận, không đổi gì.
        let fee = callback(111, "LADFYR", "update_fee", "delivered", &after(2));
        assert_eq!(deliver(&app, &hook_one, &fee).await.0, StatusCode::ACCEPTED);
        drain(&app).await;
        assert_eq!(status_of(&app, "boss-one", &one).await["status"], "picked_up");

        // ShopID không phải của tài khoản: 401, không lưu, không đổi gì.
        let forged = callback(222, "LADFYR", "switch_status", "delivered", &after(3));
        let (status, problem) = deliver(&app, &hook_one, &forged).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{problem}");
        assert_eq!(problem["code"], "shipping.webhook_signature");
        // Thân không phải JSON của GHN.
        let (status, problem) = deliver(&app, &hook_one, &json!(["x"])).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
        assert_eq!(problem["code"], "ghn.webhook_body");

        // Token tài khoản của `one` gắn vào định danh khách của `two`: không khớp tài khoản nào.
        let token_one = hook_one.rsplit('/').next().unwrap();
        let tenant_two = hook_two.rsplit('/').nth(1).unwrap();
        let mixed = format!("/api/v1/shipping/webhooks/ghn/{tenant_two}/{token_one}");
        let delivered = callback(111, "LADFYR", "switch_status", "delivered", &after(4));
        let (status, problem) = deliver(&app, &mixed, &delivered).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{problem}");
        assert_eq!(problem["code"], "shipping.webhook_unknown");
        // Định danh khách bịa.
        let unknown = format!("/api/v1/shipping/webhooks/ghn/not-a-tenant/{token_one}");
        let (status, problem) = deliver(&app, &unknown, &delivered).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{problem}");
        assert_eq!(problem["code"], "provider.webhook_unknown");
        drain(&app).await;
        assert_eq!(status_of(&app, "boss-one", &one).await["status"], "picked_up");

        // Callback thật của `two` tới đúng địa chỉ của `two`.
        let delivered_two = callback(222, "LADFYR", "switch_status", "delivered", &after(5));
        assert_eq!(deliver(&app, &hook_two, &delivered_two).await.0, StatusCode::ACCEPTED);
        drain(&app).await;
        assert_eq!(status_of(&app, "boss-two", &two).await["status"], "delivered");
        assert_eq!(status_of(&app, "boss-one", &one).await["status"], "picked_up");
    })
    .await
    .unwrap();
}
