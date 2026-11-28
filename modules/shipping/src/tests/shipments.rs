//! Hãng vận chuyển và vận đơn (bước 44 mục 3–4): tài khoản hãng không lộ thông tin đăng nhập, hãng
//! lỗi không làm hỏng báo giá, nhận lại event không tạo hai vận đơn (kể cả 16 lần đồng thời), mất kết
//! nối giữa lúc đặt, giao một phần, nhiều kiện, giao thất bại rồi hoàn hàng, huỷ sau khi lấy hàng bị
//! từ chối, webhook (chữ ký sai, trùng, sai thứ tự, token của khách khác). Cả SQLite và Postgres.

use autoxide_kernel::event::{Actor, Delivered, EventHandler};
use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;

use super::rates::{Tree, shop, zone};
use super::*;
use crate::api::fake::{FakeCarrier, SIGNATURE_HEADER, sign};
use crate::api::{CarrierRegistry, ShipmentStatus};

fn vnd(amount: i64) -> Value {
    json!({"amount": amount.to_string(), "currency": "VND"})
}

/// App có hãng giả (đăng ký qua sổ hãng sau khi dựng app) và một tài khoản hãng + phương thức
/// `carrier` ở chuỗi. Trả (app, cây, hãng giả, ID phương thức, token webhook).
async fn carrier_shop(
    test: &TestDatabase,
    auto_book: bool,
) -> (TestApp, Tree, Arc<FakeCarrier>, String, String) {
    let (app, tree) = shop(test).await;
    let fake = Arc::new(FakeCarrier::new());
    app.api::<dyn CarrierRegistry>().unwrap().register(fake.clone()).unwrap();
    let body = json!({
        "org_id": tree.chain.as_uuid(), "carrier_code": "fake", "name": "Fake express",
        "credentials": {"token": "secret-token", "webhook_secret": "hook-secret"},
    });
    let (status, account) = call(&app, "POST", "/carrier-accounts", "boss", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{account}");
    assert!(!account.to_string().contains("secret-token"), "credentials never come back");
    let account_id = account["data"]["id"].as_str().unwrap().to_owned();
    let path = account["data"]["webhook_path"].as_str().unwrap();
    let token = path.rsplit('/').next().unwrap().to_owned();
    let method = super::rates::method(
        &app,
        "boss",
        tree.chain,
        "fast",
        json!({"kind": "carrier", "carrier_account_id": account_id, "service_code": "standard", "auto_book": auto_book}),
    )
    .await;
    (app, tree, fake, method, token)
}

fn request(tree: &Tree, method: &str, order: Uuid, key: &str) -> Value {
    json!({
        "order_id": order, "order_number": "DH0001", "request_key": key,
        "org_id": tree.a.as_uuid(), "method_id": method,
        "recipient_name": "Nguyen Van An", "recipient_phone": "0912345678",
        "address": {"country": "VN", "province_code": "01", "commune_code": "00004", "line": "12 Trang Tien"},
        "items": [
            {"order_line_key": "l1", "variant_id": Uuid::now_v7(), "sku": "A", "name": "Ao", "quantity": 2, "weight_grams": 300},
            {"order_line_key": "l2", "variant_id": Uuid::now_v7(), "sku": "B", "name": "Quan", "quantity": 1, "weight_grams": 500},
        ],
        "declared_value": vnd(500_000), "cod_amount": vnd(520_000),
    })
}

async fn create(app: &TestApp, body: Value) -> Value {
    let (status, created) = call(app, "POST", "/shipments", "boss", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    created["data"].clone()
}

async fn book(app: &TestApp, id: &str) -> Value {
    let (status, booked) = call(app, "POST", &format!("/shipments/{id}/book"), "boss", None).await;
    assert_eq!(status, StatusCode::OK, "{booked}");
    booked["data"].clone()
}

async fn status(app: &TestApp, id: &str, to: &str) -> (StatusCode, Value) {
    call(app, "POST", &format!("/shipments/{id}/status"), "boss", Some(json!({"status": to}))).await
}

async fn webhook(app: &TestApp, token: &str, secret: &str, lines: Value) -> StatusCode {
    let body = lines.to_string();
    app.post(&format!("/api/v1/shipping/webhooks/fake/{token}"))
        .header(SIGNATURE_HEADER, &sign(secret, body.as_bytes()))
        .header("content-type", "application/json")
        .bytes(body.into_bytes())
        .unchecked()
        .send()
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn hang_loi_khong_lam_hong_bao_gia() {
    TestDatabase::run(async |test| {
        let (app, tree, fake, _, _) = carrier_shop(test, false).await;
        let all = zone(&app, "boss", tree.chain, "VN", json!([{"country": "VN"}])).await;
        let flat = super::rates::method(&app, "boss", tree.chain, "flat", json!({})).await;
        super::rates::rates(&app, "boss", &flat, json!([{"zone_id": all, "min_weight_grams": 0, "amount": vnd(30_000)}])).await;
        let body = json!({"org_id": tree.a.as_uuid(),
            "destination": {"country": "VN", "province_code": "01", "commune_code": "00004", "line": "1"},
            "items": [{"variant_id": Uuid::now_v7(), "quantity": 1, "weight_grams": 1500}], "order_value": vnd(1)});
        let (_, quote) = call(&app, "POST", "/quote", "boss", Some(body.clone())).await;
        let options = quote["data"]["options"].as_array().unwrap();
        assert_eq!(options.len(), 2, "{quote}");
        let carrier = options.iter().find(|option| option["method_code"] == "fast").unwrap();
        assert_eq!(carrier["amount"]["amount"], "20000", "15 000 + 5 000 for the second kilo");
        assert_eq!(carrier["carrier_code"], "fake");
        fake.set_unavailable(true);
        let (status, quote) = call(&app, "POST", "/quote", "boss", Some(body)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(quote["data"]["options"].as_array().unwrap().len(), 1);
        assert_eq!(quote["data"]["unavailable"][0]["method_code"], "fast");
        assert_eq!(quote["data"]["unavailable"][0]["reason_code"], "shipping.carrier_unavailable");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn trang_thai_kiem_tai_khoan_hang() {
    TestDatabase::run(async |test| {
        let (app, _tree, _fake, _, _) = carrier_shop(test, false).await;
        let (_, list) = call(&app, "GET", "/carrier-accounts", "boss", None).await;
        let account = &list["data"][0];
        assert_eq!(account["check_status"], "never");
        assert_eq!(account["credential_fields"], json!(["token", "webhook_secret"]));
        let id = account["id"].as_str().unwrap();
        let (_, tested) =
            call(&app, "POST", &format!("/carrier-accounts/{id}/test"), "boss", None).await;
        assert_eq!(tested["data"]["check_status"], "ok", "{tested}");
        let wrong = json!({"credentials": {"token": "wrong", "webhook_secret": "x"}});
        call(&app, "PATCH", &format!("/carrier-accounts/{id}"), "boss", Some(wrong)).await;
        let (_, tested) =
            call(&app, "POST", &format!("/carrier-accounts/{id}/test"), "boss", None).await;
        assert_eq!(tested["data"]["check_status"], "failed");
        assert_eq!(tested["data"]["check_error_code"], "shipping.carrier_unauthorized");
        // Hãng chưa cài bộ nối.
        let body = json!({"carrier_code": "ghn", "name": "GHN", "credentials": {"token": "x"}});
        let (status, problem) = call(&app, "POST", "/carrier-accounts", "boss", Some(body)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(problem["code"], "shipping.carrier_unknown");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nhan_lai_event_khong_tao_hai_van_don() {
    TestDatabase::run(async |test| {
        let (app, tree, _fake, method, _) = carrier_shop(test, false).await;
        let order = Uuid::now_v7();
        let shipment: crate::api::ShipmentRequest = {
            let body = request(&tree, &method, order, "first");
            let mut value = body.clone();
            value["org_id"] = json!(tree.a.as_uuid());
            value["origin_org_id"] = json!(tree.a.as_uuid());
            value["direction"] = json!("outbound");
            value["option_key"] = Value::Null;
            value["buyer_id"] = Value::Null;
            value["quoted_fee"] = Value::Null;
            value["note"] = json!("");
            value["address"] = json!({"country": "VN", "province_code": "01", "province_name": "Thanh pho Ha Noi",
                "commune_code": "00004", "commune_name": "Phuong Ba Dinh", "line": "12 Trang Tien"});
            for item in value["items"].as_array_mut().unwrap() {
                item["length_mm"] = Value::Null;
                item["width_mm"] = Value::Null;
                item["height_mm"] = Value::Null;
            }
            serde_json::from_value(value).unwrap()
        };
        let event = Delivered::new(
            app.default_tenant(),
            "orders",
            "orders.fulfillment_requested",
            json!({"after": {"shipment": shipment}}),
        );
        let handler = crate::listeners::Requested { service: service_of(&app), direction: crate::api::Direction::Outbound };
        handler.handle(&event).await.unwrap();
        handler.handle(&event).await.unwrap();
        let handler = Arc::new(handler);
        let mut runs = Vec::new();
        for _ in 0..16 {
            let (handler, event) = (handler.clone(), event.clone());
            runs.push(tokio::spawn(async move { handler.handle(&event).await }));
        }
        for run in runs {
            run.await.unwrap().unwrap();
        }
        let (_, list) = call(&app, "GET", &format!("/shipments?order_id={order}"), "boss", None).await;
        assert_eq!(list["data"].as_array().unwrap().len(), 1, "{list}");
        assert_eq!(list["data"][0]["status"], "pending");
        // Cùng khoá, khác nội dung: bị từ chối, không ghi đè.
        let mut other = request(&tree, &method, order, "first");
        other["items"][0]["quantity"] = json!(5);
        let (status, problem) = call(&app, "POST", "/shipments", "boss", Some(other)).await;
        assert_eq!(status, StatusCode::CONFLICT, "{problem}");
        assert_eq!(problem["code"], "shipping.request_conflict");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dat_hang_mat_ket_noi_giua_chung() {
    TestDatabase::run(async |test| {
        let (app, tree, fake, method, _) = carrier_shop(test, false).await;
        let created = create(&app, request(&tree, &method, Uuid::now_v7(), "k")).await;
        let id = created["id"].as_str().unwrap();
        fake.lose_next_create_response();
        let (status, _) = call(&app, "POST", &format!("/shipments/{id}/book"), "boss", None).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        let (_, stuck) = call(&app, "GET", &format!("/shipments/{id}"), "boss", None).await;
        assert_eq!(stuck["data"]["status"], "booking");
        // Lần sau hỏi `find_by_reference` trước: hãng đã có đơn → ghi, không tạo đơn thứ hai.
        let service = service_of(&app);
        let tenant = app.default_tenant();
        service.book(&tenant, Actor::system(), Uuid::parse_str(id).unwrap()).await.unwrap();
        assert_eq!(fake.created_count(), 1);
        let (_, booked) = call(&app, "GET", &format!("/shipments/{id}"), "boss", None).await;
        assert_eq!(booked["data"]["status"], "ready_to_ship", "{booked}");
        assert!(booked["data"]["tracking_number"].as_str().unwrap().starts_with("FAKE"));
        let (status, problem) =
            call(&app, "POST", &format!("/shipments/{id}/book"), "boss", None).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(problem["code"], "shipping.already_booked");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn giao_mot_phan_hai_van_don() {
    TestDatabase::run(async |test| {
        let (app, tree, _fake, method, _) = carrier_shop(test, false).await;
        let order = Uuid::now_v7();
        let created = create(&app, request(&tree, &method, order, "all")).await;
        let id = created["id"].as_str().unwrap();
        let (status, split) = call(
            &app,
            "POST",
            &format!("/shipments/{id}/split"),
            "boss",
            Some(json!({"lines": {"l2": 1}})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{split}");
        assert_eq!(split["data"]["items"].as_array().unwrap().len(), 1);
        assert_eq!(split["data"]["cod_amount"], Value::Null, "COD stays with the original");
        let (_, list) =
            call(&app, "GET", &format!("/shipments?order_id={order}"), "boss", None).await;
        assert_eq!(list["data"].as_array().unwrap().len(), 2);
        let (_, original) = call(&app, "GET", &format!("/shipments/{id}"), "boss", None).await;
        assert_eq!(original["data"]["items"].as_array().unwrap().len(), 1);
        // Tách hết thì không được.
        let (status, _) = call(
            &app,
            "POST",
            &format!("/shipments/{id}/split"),
            "boss",
            Some(json!({"lines": {"l1": 2}})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nhieu_kien_mot_van_don() {
    TestDatabase::run(async |test| {
        let (app, tree, _fake, method, _) = carrier_shop(test, false).await;
        let created = create(&app, request(&tree, &method, Uuid::now_v7(), "k")).await;
        let id = created["id"].as_str().unwrap();
        let parcels = json!({"parcels": [
            {"weight_grams": 650, "items": ["l1"]},
            {"weight_grams": 550, "length_mm": 300, "width_mm": 200, "height_mm": 100, "items": ["l2"]},
        ]});
        let (status, saved) = call(&app, "PUT", &format!("/shipments/{id}/parcels"), "boss", Some(parcels)).await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        let booked = book(&app, id).await;
        let parcels = booked["parcels"].as_array().unwrap();
        assert_eq!(parcels.len(), 2);
        let tracking = booked["tracking_number"].as_str().unwrap();
        assert_eq!(parcels[1]["tracking_number"], format!("{tracking}-2"));
        assert_ne!(booked["items"][0]["parcel_id"], booked["items"][1]["parcel_id"]);
        let label =
            app.get(&format!("/api/v1/shipping/shipments/{id}/label")).token("boss").send().await.unwrap();
        assert_eq!(label.status(), StatusCode::OK);
        assert_eq!(label.header("content-type"), "application/pdf");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn giao_that_bai_roi_hoan_hang() {
    TestDatabase::run(async |test| {
        let (app, tree, _fake, method, _) = carrier_shop(test, false).await;
        let id = create(&app, request(&tree, &method, Uuid::now_v7(), "k")).await["id"]
            .as_str()
            .unwrap()
            .to_owned();
        book(&app, &id).await;
        for step in [
            "picked_up",
            "in_transit",
            "out_for_delivery",
            "delivery_failed",
            "out_for_delivery",
            "delivery_failed",
            "returning",
            "returned",
        ] {
            let (code, body) = status(&app, &id, step).await;
            assert_eq!(code, StatusCode::OK, "{step}: {body}");
        }
        let (code, problem) = status(&app, &id, "delivered").await;
        assert_eq!(code, StatusCode::CONFLICT);
        assert_eq!(problem["code"], "shipping.invalid_transition");
        let (_, detail) = call(&app, "GET", &format!("/shipments/{id}"), "boss", None).await;
        assert_eq!(detail["data"]["status"], "returned");
        assert_eq!(detail["data"]["cod_status"], "pending", "nothing collected on a return");
        assert!(detail["data"]["events"].as_array().unwrap().len() >= 10);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn huy_sau_khi_lay_hang_bi_tu_choi() {
    TestDatabase::run(async |test| {
        let (app, tree, _fake, method, _) = carrier_shop(test, false).await;
        let early = create(&app, request(&tree, &method, Uuid::now_v7(), "a")).await["id"]
            .as_str()
            .unwrap()
            .to_owned();
        book(&app, &early).await;
        let (code, cancelled) =
            call(&app, "POST", &format!("/shipments/{early}/cancel"), "boss", None).await;
        assert_eq!(code, StatusCode::OK, "{cancelled}");
        assert_eq!(cancelled["data"]["status"], "cancelled");
        let late = create(&app, request(&tree, &method, Uuid::now_v7(), "b")).await["id"]
            .as_str()
            .unwrap()
            .to_owned();
        book(&app, &late).await;
        status(&app, &late, "picked_up").await;
        let (code, problem) =
            call(&app, "POST", &format!("/shipments/{late}/cancel"), "boss", None).await;
        assert_eq!(code, StatusCode::CONFLICT);
        assert_eq!(problem["code"], "shipping.cancel_too_late");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn webhook_chu_ky_trung_va_sai_thu_tu() {
    TestDatabase::run(async |test| {
        let (app, tree, _fake, method, token) = carrier_shop(test, false).await;
        let id = create(&app, request(&tree, &method, Uuid::now_v7(), "k")).await["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let tracking = book(&app, &id).await["tracking_number"].as_str().unwrap().to_owned();
        let at = Timestamp::now();
        let line = |event: &str, status: &str, minutes: i64| {
            json!({"event_id": event, "tracking_number": tracking, "status": status,
                   "occurred_at": at + SignedDuration::from_mins(minutes)})
        };
        // Chữ ký sai: 401, không lưu.
        let lines = json!([line("e1", "picked_up", 1)]);
        assert_eq!(webhook(&app, &token, "wrong", lines.clone()).await, StatusCode::UNAUTHORIZED);
        // Token không có: 404.
        assert_eq!(
            webhook(&app, "nope", "hook-secret", lines.clone()).await,
            StatusCode::NOT_FOUND
        );
        // `delivered` tới trước `in_transit`; cùng webhook gửi hai lần.
        assert_eq!(
            webhook(&app, &token, "hook-secret", json!([line("e3", "delivered", 30)])).await,
            StatusCode::ACCEPTED
        );
        assert_eq!(
            webhook(&app, &token, "hook-secret", json!([line("e2", "in_transit", 10)])).await,
            StatusCode::ACCEPTED
        );
        assert_eq!(
            webhook(&app, &token, "hook-secret", json!([line("e3", "delivered", 30)])).await,
            StatusCode::ACCEPTED
        );
        drain(&app).await;
        let (_, detail) = call(&app, "GET", &format!("/shipments/{id}"), "boss", None).await;
        assert_eq!(detail["data"]["status"], "delivered", "{detail}");
        assert_eq!(detail["data"]["cod_status"], "collected");
        let carrier_events: Vec<&Value> = detail["data"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| event["source"] == "carrier")
            .collect();
        // Đặt hãng + `in_transit` + `delivered` (e3 chỉ áp một lần, e1 sai chữ ký không lưu).
        assert_eq!(carrier_events.len(), 3, "e3 applied once: {detail}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn token_cua_khach_a_o_khach_b() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test)
            .multi_tenant()
            .tenant("a")
            .tenant("b")
            .user_in("a", "boss_a", 1, &["shipping.carriers.manage", "shipping.settings.manage"])
            .provide::<dyn AdministrativeUnits>(Arc::new(FakeAdministrativeUnits::sample()))
            .module(crate::module())
            .build()
            .await
            .unwrap();
        app.api::<dyn CarrierRegistry>().unwrap().register(Arc::new(FakeCarrier::new())).unwrap();
        let body = json!({"carrier_code": "fake", "name": "A", "credentials": {"token": "t", "webhook_secret": "s"}});
        let (status, account) = app
            .post("/api/v1/shipping/carrier-accounts")
            .header("Autoxide-Tenant", "a")
            .token("boss_a")
            .json(body)
            .send()
            .await
            .map(|answer| (answer.status(), answer.json()))
            .unwrap();
        assert_eq!(status, StatusCode::CREATED, "{account}");
        let path = account["data"]["webhook_path"].as_str().unwrap().to_owned();
        let body = "[]";
        let signature = sign("s", body.as_bytes());
        // Địa chỉ mới mang định danh của khách: hãng gọi không cần header, và header của khách khác
        // không đổi được khách nhận.
        let token = path.rsplit('/').next().unwrap().to_owned();
        assert_eq!(path.split('/').count(), 8, "the address names the tenant: {path}");
        let send = |path: String, tenant: Option<&'static str>| {
            let mut request = app
                .post(&path)
                .header(SIGNATURE_HEADER, &signature)
                .bytes(body.as_bytes().to_vec())
                .unchecked();
            if let Some(tenant) = tenant {
                request = request.header("Autoxide-Tenant", tenant);
            }
            async move { request.send().await.unwrap() }
        };
        assert_eq!(send(path.clone(), None).await.status(), StatusCode::ACCEPTED);
        assert_eq!(send(path.clone(), Some("b")).await.status(), StatusCode::ACCEPTED);
        // Định danh khách sai (kể cả của khách khác): không tìm ra khách.
        let forged = format!("/api/v1/shipping/webhooks/fake/not-a-tenant/{token}");
        let answer = send(forged, Some("a")).await;
        assert_eq!(answer.status(), StatusCode::NOT_FOUND);
        assert_eq!(answer.json()["code"], "provider.webhook_unknown");
        // Địa chỉ cũ (không có định danh khách) vẫn chạy: khách lấy từ header / tên miền, token của
        // khách A gửi vào khách B không khớp tài khoản nào.
        let old = format!("/api/v1/shipping/webhooks/fake/{token}");
        assert_eq!(send(old.clone(), Some("a")).await.status(), StatusCode::ACCEPTED);
        let answer = send(old, Some("b")).await;
        assert_eq!(answer.status(), StatusCode::NOT_FOUND);
        assert_eq!(answer.json()["code"], "shipping.webhook_unknown");
        let _ = ShipmentStatus::Pending;
    })
    .await
    .unwrap();
}

/// Rà soát bảo mật 2026-11: webhook của tài khoản hãng thứ hai (cùng hãng, chữ ký đúng của chính nó)
/// không đổi được trạng thái vận đơn đặt qua tài khoản khác dù biết mã vận đơn.
#[tokio::test]
async fn webhook_tai_khoan_khac_khong_doi_van_don() {
    TestDatabase::run(async |test| {
        let (app, tree, _fake, method, _token) = carrier_shop(test, false).await;
        let id = create(&app, request(&tree, &method, Uuid::now_v7(), "k")).await["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let tracking = book(&app, &id).await["tracking_number"].as_str().unwrap().to_owned();
        let body = json!({
            "org_id": tree.a.as_uuid(), "carrier_code": "fake", "name": "Other shop",
            "credentials": {"token": "other-token", "webhook_secret": "other-secret"},
        });
        let (status, other) = call(&app, "POST", "/carrier-accounts", "boss", Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{other}");
        let other_token =
            other["data"]["webhook_path"].as_str().unwrap().rsplit('/').next().unwrap().to_owned();
        let line = json!([{"event_id": "x1", "tracking_number": tracking, "status": "delivered",
                           "occurred_at": Timestamp::now()}]);
        assert_eq!(webhook(&app, &other_token, "other-secret", line).await, StatusCode::ACCEPTED);
        drain(&app).await;
        let (_, detail) = call(&app, "GET", &format!("/shipments/{id}"), "boss", None).await;
        assert_ne!(detail["data"]["status"], "delivered", "{detail}");
    })
    .await
    .unwrap();
}

/// P17: kiện nặng / dài quá trần, hay mọi kiện cộng lại quá trần một lần giao → 422 có mã; vận đơn giữ
/// nguyên kiện cũ.
#[tokio::test]
async fn chia_kien_vuot_tran_bao_loi() {
    TestDatabase::run(async |test| {
        let (app, tree, _fake, method, _) = carrier_shop(test, false).await;
        let created = create(&app, request(&tree, &method, Uuid::now_v7(), "k")).await;
        let id = created["id"].as_str().unwrap();
        let uri = format!("/shipments/{id}/parcels");
        let wild = json!({"parcels": [
            {"weight_grams": i64::MAX, "length_mm": i64::MAX, "items": ["l1", "l2"]},
        ]});
        let (status, problem) = call(&app, "PUT", &uri, "boss", Some(wild)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        let fields: Vec<&str> =
            problem["errors"].as_array().unwrap().iter().filter_map(|e| e["field"].as_str()).collect();
        assert_eq!(fields, ["parcels[0].weight_grams", "parcels[0].length_mm"], "{problem}");
        // Mỗi kiện đúng trần, hai kiện cộng lại vượt trần.
        let heavy = json!({"parcels": [
            {"weight_grams": 1_000_000_000_i64, "items": ["l1"]},
            {"weight_grams": 1, "items": ["l2"]},
        ]});
        let (status, problem) = call(&app, "PUT", &uri, "boss", Some(heavy)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        assert_eq!(problem["code"], "shipping.load_too_large", "{problem}");
        // Yêu cầu giao mà tổng cân các món quá trần: từ chối ngay lúc tạo.
        let mut big = request(&tree, &method, Uuid::now_v7(), "big");
        big["items"][0]["weight_grams"] = json!(10_000_000);
        big["items"][0]["quantity"] = json!(101);
        let (status, problem) = call(&app, "POST", "/shipments", "boss", Some(big)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        assert_eq!(problem["code"], "shipping.load_too_large", "{problem}");
    })
    .await
    .unwrap();
}
