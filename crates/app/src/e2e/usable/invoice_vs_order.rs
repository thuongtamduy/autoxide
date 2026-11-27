//! Hoá đơn của đơn so với số đơn đã chốt (LG-10, kiểm độc lập #575 V2–V5) trên chương trình thật — `orders`,
//! `catalog`, `shipping`, `tax`, `invoices` (cả SQLite và Postgres): phí giao trên hoá đơn giá chưa thuế, lệch làm
//! tròn tối đa 1 đồng, đơn trộn hai chế độ giá, hàng chịu hai tầng thuế (TTĐB rồi GTGT).

use autoxide_kernel::tenancy::{DEFAULT_TENANT, root_of};
use autoxide_testkit::{TestApp, TestDatabase};
use serde_json::{Value, json};

use super::checkout_more::{BUYER, activate_product, shop};
use super::kit::{ADMIN, call_tool, drain, expect, get, id};

/// Dãy số hoá đơn của năm nay (giờ Việt Nam).
async fn series(app: &TestApp) {
    let vietnam = autoxide_kernel::time::zone("Asia/Ho_Chi_Minh").unwrap();
    let year = i32::from(autoxide_kernel::time::Timestamp::now().date_in(&vietnam).year());
    let series = json!({"kind": "vat_invoice", "symbol": "1C26TAA", "year": year});
    expect(app, "POST", "/api/v1/invoices/series", ADMIN, Some(series), 201).await;
}

/// Hồ sơ thuế Việt Nam cho nút gốc. Lập SAU bảng giá chưa thuế: luật giá gồm thuế cho người tiêu dùng của `catalog`
/// chỉ áp khi đã có hồ sơ (bảng cũ vẫn dùng được).
async fn vietnam_profile(app: &TestApp) {
    let root = root_of(DEFAULT_TENANT).as_uuid();
    let profile = json!({"jurisdiction": "VN", "regime": "vat_credit"});
    expect(app, "PUT", &format!("/api/v1/tax/profiles/{root}"), ADMIN, Some(profile), 200).await;
}

/// Bảng giá chưa gồm thuế, ưu tiên hơn bảng giá gồm thuế của `shop`.
async fn net_price_list(app: &TestApp) -> String {
    let list =
        json!({"name": "Giá chưa thuế", "currency": "VND", "tax_inclusive": false, "priority": 10});
    id(&expect(app, "POST", "/api/v1/catalog/price-lists", ADMIN, Some(list), 201).await)
}

async fn price(app: &TestApp, list: &str, variant: &str, amount: i64) {
    let prices = json!({"reference": format!("p-{variant}"), "prices": [{"variant_id": variant,
        "amount": {"amount": amount.to_string(), "currency": "VND"}}]});
    expect(
        app,
        "POST",
        &format!("/api/v1/catalog/price-lists/{list}/prices"),
        ADMIN,
        Some(prices),
        200,
    )
    .await;
}

/// Sản phẩm mới (đã nhập kho, đang bán); trả ID biến thể.
async fn product(app: &TestApp, sku: &str, tax_category: &str) -> String {
    let product = json!({"kind": "physical", "tax_category": tax_category, "locale": "vi",
        "text": {"name": sku}, "variant": {"sku": sku, "weight_grams": 500}});
    let created = expect(app, "POST", "/api/v1/catalog/products", ADMIN, Some(product), 201).await;
    let variant = created["data"]["variants"][0]["id"].as_str().unwrap().to_owned();
    let root = root_of(DEFAULT_TENANT);
    let stock = json!({"org_id": root, "reference": format!("open-{sku}"), "lines": [{"variant_id": variant,
        "quantity": 20, "reason_code": "found", "unit_cost": {"amount": "1000", "currency": "VND"}}]});
    expect(app, "POST", "/api/v1/inventory/adjustments", ADMIN, Some(stock), 201).await;
    activate_product(app, &id(&created)).await;
    variant
}

/// Đổi phí giao của cách giao duy nhất của `shop`.
async fn shipping_fee(app: &TestApp, amount: i64) {
    let methods = get(app, "/api/v1/shipping/methods").await;
    let method = methods["data"][0]["id"].as_str().unwrap().to_owned();
    let zones = get(app, "/api/v1/shipping/zones").await;
    let zone = zones["data"][0]["id"].clone();
    let rates = json!({"rates": [{"zone_id": zone, "min_weight_grams": 0,
        "amount": {"amount": amount.to_string(), "currency": "VND"}}]});
    expect(
        app,
        "PUT",
        &format!("/api/v1/shipping/methods/{method}/rates"),
        ADMIN,
        Some(rates),
        200,
    )
    .await;
}

/// Người mua đặt `lines` (biến thể, số lượng), giao hàng xong; trả (đơn đã đặt, chứng từ của đơn, hoá đơn).
async fn invoiced_order(app: &TestApp, key: &str, lines: &[(&str, i64)]) -> (Value, Value, Value) {
    let cart = id(&expect(app, "POST", "/api/v1/orders/carts", BUYER, Some(json!({})), 201).await);
    let path = format!("/api/v1/orders/carts/{cart}");
    for (variant, quantity) in lines {
        expect(
            app,
            "PUT",
            &format!("{path}/lines/{variant}"),
            BUYER,
            Some(json!({"quantity": quantity})),
            200,
        )
        .await;
    }
    let mut delivery = json!({"recipient_name": "An", "phone": "0901234567",
        "address": {"country": "VN", "province_code": "01", "commune_code": "00004", "line": "12 Tràng Tiền"}});
    let priced =
        expect(app, "PUT", &format!("{path}/delivery"), BUYER, Some(delivery.clone()), 200).await;
    delivery["shipping_option"] = priced["data"]["shipping"]["options"][0]["option_key"].clone();
    let priced = expect(app, "PUT", &format!("{path}/delivery"), BUYER, Some(delivery), 200).await;
    let body = json!({"cart_id": cart, "quote_token": priced["data"]["quote_token"], "payment_method": "cod",
                      "contact": {"name": "An", "phone": "0901234567"}});
    let answer = app
        .post("/api/v1/orders/checkout")
        .token(BUYER)
        .header("idempotency-key", key)
        .json(body)
        .send()
        .await
        .unwrap();
    let placed = answer.json();
    assert_eq!(answer.status(), 201, "{placed}");
    let order = placed["data"]["orders"][0]["id"].as_str().unwrap().to_owned();
    for to in ["preparing", "shipping", "delivered"] {
        call_tool(app, ADMIN, "orders.transition_order", json!({"id": order, "to": to})).await;
    }
    drain(app).await;
    let documents = expect(
        app,
        "GET",
        &format!("/api/v1/orders/me/orders/{order}/documents"),
        BUYER,
        None,
        200,
    )
    .await;
    let document = documents["data"][0]["document_id"]
        .as_str()
        .unwrap_or_else(|| panic!("{documents}"))
        .to_owned();
    let invoice = get(app, &format!("/api/v1/invoices/documents/{document}")).await;
    (placed["data"]["orders"][0].clone(), documents["data"][0].clone(), invoice["data"].clone())
}

fn amount(value: &Value) -> i64 {
    value["amount"].as_str().unwrap().parse().unwrap()
}

/// Giá chưa thuế, phí giao 30.000 đ (V2): hoá đơn ghi phí giao bằng phần trước thuế, tổng hoá đơn bằng tổng khách
/// trả; phí 28.000 đ (V3): hoá đơn lệch tối đa 1 đồng do làm tròn, cờ vẫn `false`.
#[tokio::test]
async fn gia_chua_thue_co_phi_giao_hoa_don_khop_don() {
    TestDatabase::run(async |test| {
        let (app, _gateway, variant, _) = shop(test, &[]).await;
        series(&app).await;
        let list = net_price_list(&app).await;
        price(&app, &list, &variant, 150_000).await;
        vietnam_profile(&app).await;

        let (order, document, invoice) = invoiced_order(&app, "net-1", &[(&variant, 1)]).await;
        // 150.000 + 10% + phí giao 30.000 đã gồm thuế.
        assert_eq!(amount(&order["totals"]["total"]), 195_000, "{order}");
        assert_eq!(document["differs_from_order"], false, "{document}");
        assert_eq!(amount(&document["total"]), 195_000, "{document}");
        assert_eq!(invoice["prices_include_tax"], false, "{invoice}");
        let shipping = invoice["lines"]
            .as_array()
            .unwrap()
            .iter()
            .find(|line| line["item_ref"].is_null())
            .cloned();
        assert_eq!(
            amount(&shipping.unwrap_or_else(|| panic!("{invoice}"))["total"]),
            30_000,
            "{invoice}"
        );

        shipping_fee(&app, 28_000).await;
        let (order, document, invoice) = invoiced_order(&app, "net-2", &[(&variant, 1)]).await;
        assert_eq!(amount(&order["totals"]["total"]), 193_000, "{order}");
        assert!((amount(&invoice["total"]) - 193_000).abs() <= 1, "{invoice}");
        assert_eq!(document["differs_from_order"], false, "{document}");
    })
    .await
    .unwrap();
}

/// Đơn trộn dòng giá chưa thuế (A 150.000) và gồm thuế (B 110.000), GTGT 10% (V5): hoá đơn chỉ có một chế độ giá
/// nên không khớp đơn — cờ luôn `true`.
#[tokio::test]
async fn don_tron_hai_che_do_gia_bao_khac() {
    TestDatabase::run(async |test| {
        let (app, _gateway, net_variant, gross_list) = shop(test, &[]).await;
        series(&app).await;
        let list = net_price_list(&app).await;
        price(&app, &list, &net_variant, 150_000).await;
        let gross_variant = product(&app, "CUP-1", "vat.standard").await;
        price(&app, &gross_list, &gross_variant, 110_000).await;
        vietnam_profile(&app).await;

        let (order, document, _) =
            invoiced_order(&app, "mixed-1", &[(&net_variant, 1), (&gross_variant, 1)]).await;
        // 165.000 + 110.000 + phí giao 30.000.
        assert_eq!(amount(&order["totals"]["total"]), 305_000, "{order}");
        assert_eq!(document["differs_from_order"], true, "{document}");
    })
    .await
    .unwrap();
}

/// Hàng chịu hai tầng thuế — bia: TTĐB rồi GTGT trên giá đã có TTĐB (V4): thuế dòng hoá đơn bằng thuế dòng đơn, cờ
/// `false` (so tiền thuế, không so suất GTGT).
#[tokio::test]
async fn hang_hai_tang_thue_khong_bao_nham() {
    TestDatabase::run(async |test| {
        let (app, _gateway, _, gross_list) = shop(test, &[]).await;
        series(&app).await;
        let beer = product(&app, "BEER-1", "excise.beer").await;
        price(&app, &gross_list, &beer, 181_500).await;
        vietnam_profile(&app).await;

        let (order, document, invoice) = invoiced_order(&app, "beer-1", &[(&beer, 1)]).await;
        let line = invoice["lines"]
            .as_array()
            .unwrap()
            .iter()
            .find(|line| line["item_ref"] == "BEER-1")
            .cloned();
        let line = line.unwrap_or_else(|| panic!("{invoice}"));
        let order_line = order["lines"][0].clone();
        assert!(amount(&order_line["tax"]) > 0, "{order}");
        assert!(
            (amount(&line["tax"]) - amount(&order_line["tax"])).abs() <= 1,
            "{invoice} {order}"
        );
        assert_eq!(document["differs_from_order"], false, "{document}");
    })
    .await
    .unwrap();
}
