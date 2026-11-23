//! Bản nháp AI của người bán dùng được như thật (cả SQLite và Postgres), với nhà cung cấp dạng OpenAI trỏ
//! tới máy chủ giả như `ai_assist`: câu trả lời gợi ý cho phiếu hỗ trợ và cho hội thoại, câu trả lời tự động
//! đã duyệt cho khách vãng lai (và phản hồi "có giúp không"), mô tả sản phẩm, tóm tắt đánh giá bị từ chối.

use autoxide_kernel::access::Principal;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, root_of};
use autoxide_testkit::{FakeProviderServer, TestApp, TestDatabase};
use serde_json::{Value, json};
use uuid::Uuid;

use super::ai_assist::{ai_answers, app_with_ai_and};
use super::kit::{ADMIN, ANONYMOUS, call_tool, drain, expect, get, id, import_reference_data};

const BUYER: &str = "buyer";
/// Đường gửi tin của eSMS (tài liệu công khai của eSMS).
const ESMS_SEND: &str = "/MainService.svc/json/SendMultipleMessage_V4_post_json/";

fn vnd(amount: i64) -> Value {
    json!({"amount": amount.to_string(), "currency": "VND"})
}

async fn app(
    test: &TestDatabase,
    server: &FakeProviderServer,
    settings: &[(&str, &str)],
) -> TestApp {
    app_with_ai_and(test, server, settings, |builder| {
        builder.principal(BUYER, Principal::customer(Uuid::from_u128(0xb3)))
    })
    .await
}

async fn write(
    app: &TestApp,
    method: &str,
    path: &str,
    body: Option<Value>,
    expected: u16,
) -> Value {
    let method = axum::http::Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method.clone(), path).token(ADMIN).header("if-match", "*");
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let json: Value = serde_json::from_slice(answer.body()).unwrap_or(Value::Null);
    assert_eq!(answer.status().as_u16(), expected, "{method} {path}: {json}");
    json
}

#[tokio::test]
async fn goi_y_tra_loi_phieu_hoi_thoai_va_tra_loi_tu_dong_da_duyet() {
    TestDatabase::run(async |test| {
        let server = FakeProviderServer::start().await.unwrap();
        let app = app(test, &server, &[]).await;
        let root = json!(root_of(DEFAULT_TENANT));

        // Câu trả lời mẫu được duyệt để trả tự động; chính sách bật phiếu vãng lai và trả tự động.
        let saved = json!({"name": "Giờ mở cửa", "org_id": root, "bodies": {"vi": "Shop mở cửa 8:00 - 21:00 mỗi ngày."}});
        let saved = id(&expect(&app, "POST", "/api/v1/support/macros", ADMIN, Some(saved), 201).await);
        let review_on = (autoxide_kernel::time::Timestamp::now() + jiff::SignedDuration::from_hours(24 * 60)).date_utc();
        let approval = json!({"enabled": true, "questions": {"vi": ["Mấy giờ shop mở cửa?"]}, "review_due_on": review_on.to_string()});
        write(&app, "PUT", &format!("/api/v1/support/macros/{saved}/auto-reply"), Some(approval), 200).await;
        let policy_path = format!("/api/v1/support/policies/{}", root.as_str().unwrap());
        let mut policy = get(&app, &policy_path).await["data"].clone();
        for key in ["org_id", "inherited", "source", "updated_at", "version", "defined_at"] {
            policy.as_object_mut().unwrap().remove(key);
        }
        policy["guest_tickets"] = json!(true);
        policy["auto_reply"] = json!(true);
        write(&app, "PUT", &policy_path, Some(policy), 200).await;
        call_tool(&app, ADMIN, "support.preview_auto_reply", json!({"org_id": root, "text": "Mấy giờ mở cửa?"})).await;

        // Khách vãng lai hỏi; bộ phân loại (AI) chọn câu trả lời đã duyệt; khách báo câu trả lời có ích.
        ai_answers(&server, json!({"kind": "answer", "macro_id": saved, "confidence_bp": 9800, "risk_flags": []}));
        let form = expect(&app, "GET", "/api/v1/antispam/token?scope=support:tickets", ANONYMOUS, None, 200).await;
        let opened = json!({"category": "other", "subject": "Giờ mở cửa", "body": "Mấy giờ shop mở cửa?", "name": "An",
                            "email": "an@example.com", "form_token": form["data"]["token"]});
        let opened = expect(&app, "POST", "/api/v1/support/public/tickets", ANONYMOUS, Some(opened), 201).await;
        let ticket = id(&opened);
        let token = opened["data"]["access_token"].clone();
        drain(&app).await;
        let read = app
            .get(&format!("/api/v1/support/public/tickets/{ticket}?token={}", token.as_str().unwrap()))
            .send()
            .await
            .unwrap();
        let read = read.json();
        let reply = read["data"]["messages"]
            .as_array()
            .and_then(|messages| messages.iter().find(|message| message["auto_reply"] == true))
            .cloned()
            .unwrap_or_else(|| panic!("no automatic answer in {read}"));
        let reply_id = reply["id"].as_str().unwrap();
        let feedback = json!({"helpful": true, "token": token});
        expect(&app, "POST", &format!("/api/v1/support/public/tickets/{ticket}/auto-replies/{reply_id}/feedback"), ANONYMOUS, Some(feedback), 200).await;

        // Nhân viên xin bản nháp trả lời cho phiếu.
        ai_answers(&server, json!({"reply": "Chào An, shop mở cửa từ 8 giờ sáng."}));
        call_tool(&app, ADMIN, "support.suggest_reply", json!({"id": ticket})).await;

        // Hội thoại: người mua nhắn, nhân viên xin bản nháp.
        let started = json!({"body": "Áo này còn size M không?", "client_id": "c-1"});
        let started = expect(&app, "POST", "/api/v1/conversations/me/threads", BUYER, Some(started), 201).await;
        let thread = started["data"]["thread"]["id"].as_str().unwrap().to_owned();
        ai_answers(&server, json!({"reply": "Còn size M bạn nhé."}));
        call_tool(&app, ADMIN, "conversations.suggest_reply", json!({"id": thread})).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn mo_ta_san_pham_va_tom_tat_danh_gia_bi_tu_choi() {
    TestDatabase::run(async |test| {
        let server = FakeProviderServer::start().await.unwrap();
        let esms = FakeProviderServer::start().await.unwrap();
        let app = app(test, &server, &[("AUTOXIDE_ESMS_API_URL", esms.url())]).await;
        import_reference_data(&app).await;
        let root = json!(root_of(DEFAULT_TENANT));

        let product = json!({"kind": "physical", "tax_category": "vat.standard", "locale": "vi",
            "text": {"name": "Ấm trà"}, "variant": {"sku": "TEA-1", "weight_grams": 600}});
        let created = expect(&app, "POST", "/api/v1/catalog/products", ADMIN, Some(product), 201).await;
        let product = id(&created);
        let variant = created["data"]["variants"][0]["id"].as_str().unwrap().to_owned();
        ai_answers(&server, json!({"short_description": "Ấm trà gốm", "description": "Ấm trà gốm men lam.",
                                   "meta_title": "Ấm trà gốm", "meta_description": "Ấm trà gốm men lam"}));
        call_tool(&app, ADMIN, "catalog.suggest_copy", json!({"product_id": product, "locale": "vi"})).await;

        // Một đơn đã giao để người mua đánh giá.
        let zone = json!({"org_id": root, "name": "Toàn quốc", "regions": [{"country": "VN"}]});
        let zone = id(&expect(&app, "POST", "/api/v1/shipping/zones", ADMIN, Some(zone), 201).await);
        let method = json!({"org_id": root, "code": "standard", "kind": "table_rate", "currency": "VND",
                            "names": {"vi": "Giao tiêu chuẩn", "en": "Standard"}});
        let method = id(&expect(&app, "POST", "/api/v1/shipping/methods", ADMIN, Some(method), 201).await);
        let rates = json!({"rates": [{"zone_id": zone, "min_weight_grams": 0, "amount": vnd(30_000)}]});
        expect(&app, "PUT", &format!("/api/v1/shipping/methods/{method}/rates"), ADMIN, Some(rates), 200).await;
        let list = json!({"name": "Bán lẻ", "currency": "VND", "tax_inclusive": true});
        let list = id(&expect(&app, "POST", "/api/v1/catalog/price-lists", ADMIN, Some(list), 201).await);
        let prices = json!({"reference": "opening", "prices": [{"variant_id": variant, "amount": vnd(150_000)}]});
        expect(&app, "POST", &format!("/api/v1/catalog/price-lists/{list}/prices"), ADMIN, Some(prices), 200).await;
        let stock = json!({"org_id": root, "reference": "opening-1", "lines": [{"variant_id": variant,
            "quantity": 5, "reason_code": "found", "unit_cost": vnd(90_000)}]});
        expect(&app, "POST", "/api/v1/inventory/adjustments", ADMIN, Some(stock), 201).await;
        write(&app, "POST", &format!("/api/v1/catalog/products/{product}/status"), Some(json!({"status": "active"})), 200).await;

        let cart = id(&expect(&app, "POST", "/api/v1/orders/carts", BUYER, Some(json!({})), 201).await);
        let path = format!("/api/v1/orders/carts/{cart}");
        expect(&app, "PUT", &format!("{path}/lines/{variant}"), BUYER, Some(json!({"quantity": 1})), 200).await;
        let mut delivery = json!({"recipient_name": "An", "phone": "0901234567",
            "address": {"country": "VN", "province_code": "01", "commune_code": "00004", "line": "12 Tràng Tiền"}});
        let priced = expect(&app, "PUT", &format!("{path}/delivery"), BUYER, Some(delivery.clone()), 200).await;
        delivery["shipping_option"] = priced["data"]["shipping"]["options"][0]["option_key"].clone();
        let priced = expect(&app, "PUT", &format!("{path}/delivery"), BUYER, Some(delivery), 200).await;
        let body = json!({"cart_id": cart, "quote_token": priced["data"]["quote_token"], "payment_method": "cod",
                          "contact": {"name": "An", "phone": "0901234567"}});
        let answer = app.post("/api/v1/orders/checkout").token(BUYER).header("idempotency-key", "review-1").json(body).send().await.unwrap();
        let placed = answer.json();
        assert_eq!(answer.status(), 201, "{placed}");
        let order = placed["data"]["orders"][0]["id"].as_str().unwrap().to_owned();
        let line = placed["data"]["orders"][0]["lines"][0]["id"].as_str().unwrap().to_owned();
        for to in ["preparing", "shipping", "delivered"] {
            call_tool(&app, ADMIN, "orders.transition_order", json!({"id": order, "to": to})).await;
        }
        drain(&app).await;
        // Người viết đánh giá phải xác minh số điện thoại (NĐ 147/2024): cửa hàng bật SMS (eSMS, máy chủ
        // giả), người mua nhận mã qua SMS rồi xác nhận.
        for (field, value) in [("api_key", "esms-key"), ("secret_key", "esms-secret"), ("brandname", "AMTRA")] {
            let path = format!("/api/v1/kernel/settings/esms.connection.sms.{field}");
            expect(&app, "PUT", &path, ADMIN, Some(json!({"value": value})), 200).await;
        }
        esms.reply("POST", ESMS_SEND, 200, json!({"CodeResult": "100", "CountRegenerate": 0, "SMSID": "sms-1"}));
        let sent = json!({"phone": "0901234567"});
        let sent = expect(&app, "POST", "/api/v1/compliance/me/phone-verifications", BUYER, Some(sent), 201).await;
        drain(&app).await;
        let texts: Vec<String> = esms
            .take_requests()
            .iter()
            .filter_map(|request| request.json()["Content"].as_str().map(str::to_owned))
            .collect();
        let code: String = texts
            .last()
            .unwrap_or_else(|| panic!("no SMS sent"))
            .split(|c: char| !c.is_ascii_digit())
            .find(|word| word.len() == 6)
            .unwrap_or_else(|| panic!("no code in {texts:?}"))
            .to_owned();
        let confirm = format!("/api/v1/compliance/me/phone-verifications/{}/confirm", id(&sent));
        expect(&app, "POST", &confirm, BUYER, Some(json!({"code": code})), 200).await;
        let review = json!({"rating": 5, "body": "Ấm đẹp, giao nhanh."});
        expect(&app, "POST", &format!("/api/v1/orders/me/orders/{order}/lines/{line}/review"), BUYER, Some(review), 201).await;
        drain(&app).await;

        ai_answers(&server, json!({"summary": "Người mua khen ấm đẹp và giao nhanh.", "themes": ["đẹp", "giao nhanh"]}));
        let summary = json!({"locale": "vi"});
        let summary = expect(&app, "POST", &format!("/api/v1/orders/products/{product}/review-summaries"), ADMIN, Some(summary), 201).await;
        drain(&app).await;
        let reason = json!({"reason": "Chưa đủ đánh giá"});
        expect(&app, "POST", &format!("/api/v1/orders/review-summaries/{}/reject", id(&summary)), ADMIN, Some(reason), 200).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ai_dung_trang_sua_trang_va_soan_chien_dich_tu_khuyen_mai() {
    TestDatabase::run(async |test| {
        let server = FakeProviderServer::start().await.unwrap();
        let app = app(test, &server, &[]).await;
        let root = json!(root_of(DEFAULT_TENANT));

        // Nhờ AI dựng bố cục một trang từ mô tả, rồi sửa bằng lời. Cả hai chạy nền, kết quả là bản nháp.
        let page = json!({"kind": "page", "title": "Trà Tết"});
        let page = id(&expect(&app, "POST", "/api/v1/content/entries", ADMIN, Some(page), 201).await);
        let compose = json!({"id": page, "instruction": "Trang giới thiệu bộ quà trà Tết, có ảnh lớn và nút mua."});
        let layout = json!({"type": "layout", "schema": 1, "sections": [
            {"id": "hero", "block": "content.hero", "props": {"heading": "Quà trà Tết", "text": "Bộ quà gói sẵn."}}]});
        ai_answers(&server, json!({"layout_json": layout.to_string()}));
        let turn = call_tool(&app, ADMIN, "content.compose_page", compose).await;
        assert!(turn["id"].is_string(), "{turn}");
        drain(&app).await;
        let patch = json!([{"op": "replace", "path": "/sections/0/props/heading", "value": "Quà trà Tết đỏ"}]);
        ai_answers(&server, json!({"patch_json": patch.to_string()}));
        let edit = json!({"id": page, "instruction": "Đổi màu phần đầu trang sang đỏ."});
        call_tool(&app, ADMIN, "content.edit_page", edit).await;

        // Soạn nháp chiến dịch (trang đích) từ một khuyến mại; đọc lại yêu cầu.
        let (starts, ends) = super::kit::promotion_window();
        let promotion = json!({"org_id": root, "kind": "percent_off", "currency": "VND", "value_bp": 1000,
            "trigger": "automatic", "locale": "vi", "title": "Giảm 10% trà Tết", "terms": "Áp dụng mọi đơn",
            "starts_at": starts, "ends_at": ends});
        let promotion = call_tool(&app, ADMIN, "promotions.create_promotion", json!({"promotion": promotion})).await;
        let promotion = promotion["id"].as_str().unwrap_or_else(|| panic!("{promotion}")).to_owned();
        let request = json!({"promotion_id": promotion, "channels": ["page"], "instruction": "Giọng ấm áp."});
        let drafts = call_tool(&app, ADMIN, "content.create_campaign_drafts", request).await;
        let drafts = drafts["id"].as_str().unwrap_or_else(|| panic!("{drafts}")).to_owned();
        let read = call_tool(&app, ADMIN, "content.get_campaign_drafts", json!({"id": drafts})).await;
        assert_eq!(read["id"], drafts.as_str(), "{read}");
    })
    .await
    .unwrap();
}
