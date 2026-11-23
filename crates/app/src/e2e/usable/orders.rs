//! Bán hàng giao tận nơi dùng được như người mua và người bán thật (cả SQLite và Postgres): giỏ của người
//! mua đã đăng nhập với voucher trong ví, cách trả, nguồn giới thiệu; đặt hàng thu khi giao; nhân viên
//! nhận đơn, xin vận đơn, cập nhật trạng thái giao, xuất hoá đơn; người mua trả hàng; bảo hành; hoàn tiền.

use autoxide_kernel::access::Principal;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, root_of};
use autoxide_testkit::{TestApp, TestDatabase};
use serde_json::{Value, json};
use uuid::Uuid;

use super::kit::{
    ADMIN, app_with, call_tool, drain, expect, get, id, import_reference_data, promotion_window,
};

const BUYER: &str = "buyer";
const BUYER_ID: u128 = 0xb0;

fn vnd(amount: i64) -> Value {
    json!({"amount": amount.to_string(), "currency": "VND"})
}

/// Cửa hàng: giao tiêu chuẩn 30.000đ toàn quốc, ấm trà 150.000đ (bảo hành 12 tháng) còn 20 cái.
/// Trả (app, ID biến thể).
async fn shop(test: &TestDatabase) -> (TestApp, String) {
    let app = app_with(test, |builder| {
        builder.principal(BUYER, Principal::customer(Uuid::from_u128(BUYER_ID)))
    })
    .await;
    import_reference_data(&app).await;
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

    let product = json!({"kind": "physical", "tax_category": "vat.standard", "locale": "vi", "warranty_months": 12,
        "text": {"name": "Ấm trà"}, "variant": {"sku": "TEA-1", "weight_grams": 600}});
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
    let answer = app
        .post(&format!("/api/v1/catalog/products/{product}/status"))
        .token(ADMIN)
        .header("if-match", "*")
        .json(json!({"status": "active"}))
        .send()
        .await
        .unwrap();
    assert_eq!(answer.status(), 200, "{}", answer.text());
    (app, variant)
}

/// Một voucher 10% của chương trình đang chạy, cấp vào ví người mua; trả ID voucher trong ví.
async fn voucher_in_wallet(app: &TestApp) -> String {
    let (starts, ends) = promotion_window();
    let promotion = json!({"promotion": {"org_id": root_of(DEFAULT_TENANT), "kind": "percent_off",
        "currency": "VND", "value_bp": 1000, "trigger": "voucher", "locale": "vi", "title": "Giảm 10%",
        "terms": "Một lần mỗi khách", "starts_at": starts, "ends_at": ends}});
    let promotion = call_tool(app, ADMIN, "promotions.create_promotion", promotion).await;
    let promotion = promotion["id"].as_str().unwrap_or_else(|| panic!("{promotion}")).to_owned();
    activate(app, &promotion).await;
    let grant = json!({"promotion_id": promotion, "user_id": Uuid::from_u128(BUYER_ID), "reason": "Khách thân thiết"});
    call_tool(app, ADMIN, "promotions.grant_voucher", grant).await;
    let wallet = expect(app, "GET", "/api/v1/promotions/me/vouchers", BUYER, None, 200).await;
    wallet["data"][0]["id"].as_str().unwrap_or_else(|| panic!("{wallet}")).to_owned()
}

/// Ghi nhận thông báo với cơ quan (khi luật đòi) rồi bật chương trình bằng tool.
pub(crate) async fn activate(app: &TestApp, promotion: &str) {
    let path = format!("/api/v1/promotions/promotions/{promotion}");
    let checks = expect(app, "POST", &format!("{path}/activation-check"), ADMIN, None, 200).await;
    for check in checks["data"]["checks"].as_array().cloned().unwrap_or_default() {
        if check["code"] == "promotions.notice_required" {
            let authority = check["params"]["authority"].as_str().unwrap();
            let notice = json!({"status": "submitted", "reference": "TB-1"});
            expect(app, "PATCH", &format!("{path}/notices/{authority}"), ADMIN, Some(notice), 200)
                .await;
        }
    }
    call_tool(app, ADMIN, "promotions.activate_promotion", json!({"id": promotion})).await;
}

#[tokio::test]
async fn nguoi_mua_dat_thu_khi_giao_nhan_hang_tra_hang_va_bao_hanh() {
    TestDatabase::run(async |test| {
        let (app, variant) = shop(test).await;
        let voucher = voucher_in_wallet(&app).await;

        // Giỏ của người mua đã đăng nhập.
        let cart = id(&expect(&app, "POST", "/api/v1/orders/carts", BUYER, Some(json!({})), 201).await);
        let cart_path = format!("/api/v1/orders/carts/{cart}");
        expect(&app, "PUT", &format!("{cart_path}/lines/{variant}"), BUYER, Some(json!({"quantity": 2})), 200).await;
        // Địa chỉ trước, rồi chọn một cách giao trong các lựa chọn máy chủ báo.
        let mut delivery = json!({"recipient_name": "Nguyễn An", "phone": "0901234567",
            "address": {"country": "VN", "province_code": "01", "commune_code": "00004", "line": "12 Tràng Tiền"}});
        let priced = expect(&app, "PUT", &format!("{cart_path}/delivery"), BUYER, Some(delivery.clone()), 200).await;
        delivery["shipping_option"] = priced["data"]["shipping"]["options"][0]["option_key"].clone();
        assert!(delivery["shipping_option"].is_string(), "{priced}");
        expect(&app, "PUT", &format!("{cart_path}/delivery"), BUYER, Some(delivery), 200).await;
        expect(&app, "PUT", &format!("{cart_path}/vouchers"), BUYER, Some(json!({"voucher_ids": [voucher]})), 200).await;
        expect(&app, "PUT", &format!("{cart_path}/attribution"), BUYER, Some(json!({"click_id": Uuid::now_v7()})), 200).await;
        let cart = expect(&app, "PUT", &format!("{cart_path}/payment-method"), BUYER, Some(json!({"payment_method": "cod"})), 200).await;
        let quote = cart["data"]["quote_token"].as_str().unwrap().to_owned();

        let body = json!({"cart_id": id(&cart), "quote_token": quote, "payment_method": "cod",
                          "contact": {"name": "Nguyễn An", "phone": "0901234567"}});
        let answer = app
            .post("/api/v1/orders/checkout")
            .token(BUYER)
            .header("idempotency-key", "usable-order-1")
            .json(body)
            .send()
            .await
            .unwrap();
        let placed = answer.json();
        assert_eq!(answer.status(), 201, "{placed}");
        let order = placed["data"]["orders"][0]["id"].as_str().unwrap().to_owned();
        let line = placed["data"]["orders"][0]["lines"][0]["id"].clone();

        // Người mua xin hoá đơn công ty qua cửa hàng trước khi giao (hoá đơn xuất khi giao xong).
        let invoice = json!({"name": "Công ty An", "tax_code": "0101234567", "email": "ketoan@example.com"});
        expect(&app, "POST", &format!("/api/v1/orders/{order}/invoice"), ADMIN, Some(invoice), 200).await;
        // Nhân viên: xem, nhận đơn, xin vận đơn, giao xong.
        call_tool(&app, ADMIN, "orders.list_orders", json!({})).await;
        call_tool(&app, ADMIN, "orders.get_order", json!({"id": order})).await;
        call_tool(&app, ADMIN, "orders.transition_order", json!({"id": order, "to": "preparing"})).await;
        call_tool(&app, ADMIN, "orders.request_shipment", json!({"id": order})).await;
        drain(&app).await;
        let shipments = get(&app, &format!("/api/v1/shipping/shipments?order_id={order}")).await;
        let shipment = shipments["data"][0]["id"].as_str().unwrap_or_else(|| panic!("{shipments}")).to_owned();
        call_tool(&app, ADMIN, "shipping.get_shipment", json!({"id": shipment})).await;
        for status in ["ready_to_ship", "picked_up", "in_transit", "delivered"] {
            call_tool(&app, ADMIN, "shipping.update_status", json!({"id": shipment, "status": status})).await;
        }
        drain(&app).await;

        // Người mua trả một cái; cửa hàng duyệt.
        let request = json!({"lines": [{"line_id": line, "quantity": 1}], "reason_code": "changed_mind", "note": "Mua nhầm màu"});
        let returned = expect(&app, "POST", &format!("/api/v1/orders/me/orders/{order}/returns"), BUYER, Some(request), 201).await;
        let returned = id(&returned);
        get_as(&app, BUYER, &format!("/api/v1/orders/me/returns/{returned}")).await;
        get(&app, &format!("/api/v1/orders/returns/{returned}")).await;
        call_tool(&app, ADMIN, "orders.list_returns", json!({})).await;
        call_tool(&app, ADMIN, "orders.decide_return", json!({"id": returned, "action": "approve"})).await;

        let delivered = get(&app, &format!("/api/v1/orders/{order}")).await;
        assert_eq!(delivered["data"]["status"], "delivered", "{delivered}");

        // Cửa hàng chưa có dãy số hoá đơn: tự xuất lúc giao không được (chứng từ nháp chờ), nhưng việc
        // khác của đơn giao xong vẫn chạy (bảo hành ở dưới). Tạo dãy số rồi xin lại thì phát hành.
        let year = i32::from(autoxide_kernel::time::Timestamp::now().date_utc().year());
        let series = json!({"kind": "vat_invoice", "symbol": "1C26TAA", "year": year});
        expect(&app, "POST", "/api/v1/invoices/series", ADMIN, Some(series), 201).await;
        let invoice = json!({"name": "Công ty An", "tax_code": "0101234567", "email": "ketoan@example.com"});
        let issued = expect(&app, "POST", &format!("/api/v1/orders/{order}/invoice"), ADMIN, Some(invoice), 200).await;
        assert!(issued["data"].is_object(), "{issued}");
        // Bảo hành theo số điện thoại người mua.
        let found = call_tool(&app, ADMIN, "orders.lookup_warranty", json!({"phone": "0901234567"})).await;
        assert!(found.as_array().is_some_and(|rows| !rows.is_empty()), "{found}");
        get(&app, "/api/v1/orders/warranties?phone=0901234567").await;
        let warranty = found[0]["id"].as_str().unwrap_or_else(|| panic!("{found}")).to_owned();
        let claim = json!({"warranty_id": warranty, "org_id": root_of(DEFAULT_TENANT), "issue": "Rò nước"});
        let claim = id(&expect(&app, "POST", "/api/v1/orders/warranty-claims", ADMIN, Some(claim), 201).await);
        call_tool(&app, ADMIN, "orders.update_warranty_claim", json!({"id": claim, "to": "inspecting"})).await;

        // Hoàn một phần tiền đơn đã thu khi giao.
        let refund = json!({"id": order, "amount": "10000", "currency": "VND", "reason": "Giao trễ", "reference": "rf-1"});
        call_tool(&app, ADMIN, "orders.refund_order", refund).await;

        get(&app, "/api/v1/orders/risk/cod-rules").await;
        get(&app, "/api/v1/orders/risk/cod-blocks").await;
    })
    .await
    .unwrap();
}

async fn get_as(app: &TestApp, who: &str, path: &str) -> Value {
    expect(app, "GET", path, who, None, 200).await
}
