//! Khuyến mại dùng được như người bán thật (cả SQLite và Postgres): chương trình mã giảm giá từ bản nháp
//! (thử trên giỏ mẫu, mã sinh hàng loạt, tải CSV, dữ liệu thông báo với cơ quan), bật, tạm dừng, chạy
//! lại, kết thúc; flash sale của chuỗi với cửa hàng đăng ký bị từ chối; quy tắc thông báo theo ngày.

use autoxide_kernel::tenancy::{DEFAULT_TENANT, root_of};
use autoxide_testkit::{TestApp, TestDatabase};
use serde_json::{Value, json};

use super::kit::{ADMIN, app, call_tool, expect, get, id, import_reference_data, promotion_window};
use super::orders::activate;

fn promotion(org: &Value, extra: Value) -> Value {
    let (starts, ends) = promotion_window();
    let mut body = json!({"org_id": org, "kind": "percent_off", "currency": "VND", "value_bp": 1000,
        "locale": "vi", "title": "Giảm 10%", "terms": "Áp dụng toàn cửa hàng",
        "starts_at": starts, "ends_at": ends});
    for (key, value) in extra.as_object().unwrap() {
        body[key] = value.clone();
    }
    body
}

async fn product(app: &TestApp, org: &Value, sku: &str) -> String {
    let product = json!({"org_id": org, "kind": "physical", "tax_category": "vat.standard", "locale": "vi",
        "text": {"name": sku}, "variant": {"sku": sku, "weight_grams": 500}});
    let created = expect(app, "POST", "/api/v1/catalog/products", ADMIN, Some(product), 201).await;
    created["data"]["variants"][0]["id"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn ma_giam_gia_tu_nhap_toi_ket_thuc() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        import_reference_data(&app).await;
        let root = json!(root_of(DEFAULT_TENANT));
        let variant = product(&app, &root, "TEA-1").await;
        get(&app, "/api/v1/promotions/notice-rules").await;

        let draft = promotion(&root, json!({"trigger": "code"}));
        let created = call_tool(&app, ADMIN, "promotions.create_promotion", json!({"promotion": draft})).await;
        let promotion = created["id"].as_str().unwrap_or_else(|| panic!("{created}")).to_owned();
        call_tool(&app, ADMIN, "promotions.get_promotion", json!({"id": promotion})).await;
        call_tool(&app, ADMIN, "promotions.list_promotions", json!({})).await;
        let cart = json!({"org_id": root, "currency": "VND", "buyer_keys": [], "customer_groups": [],
            "first_order": false, "codes": [], "voucher_ids": [], "gift_selections": [],
            "at": "2026-06-01T00:00:00Z", "locale": "vi",
            "lines": [{"line_key": "l0", "variant_id": variant, "quantity": 2, "has_compare_at": false,
                       "unit_price": {"amount": "200000", "currency": "VND"}}]});
        call_tool(&app, ADMIN, "promotions.simulate", json!({"id": promotion, "carts": [cart]})).await;
        call_tool(&app, ADMIN, "promotions.export_notice", json!({"id": promotion})).await;
        let codes = json!({"id": promotion, "purpose": "redeem", "count": 3, "prefix": "TET"});
        call_tool(&app, ADMIN, "promotions.generate_codes", codes).await;
        let path = format!("/api/v1/promotions/promotions/{promotion}");
        let listed = get(&app, &format!("{path}/codes")).await;
        assert_eq!(listed["data"].as_array().unwrap().len(), 3, "{listed}");
        let csv = app.get(&format!("{path}/codes/export")).token(ADMIN).send().await.unwrap();
        assert_eq!(csv.status(), 200);
        assert!(csv.text().contains("TET"), "{}", csv.text());

        activate(&app, &promotion).await;
        expect(&app, "POST", &format!("{path}/pause"), ADMIN, None, 200).await;
        expect(&app, "POST", &format!("{path}/resume"), ADMIN, None, 200).await;
        call_tool(&app, ADMIN, "promotions.end_promotion", json!({"id": promotion})).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn flash_sale_cua_chuoi_tu_choi_cua_hang_dang_ky() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        import_reference_data(&app).await;
        let root = json!(root_of(DEFAULT_TENANT));
        expect(&app, "POST", "/api/v1/organizations/unit-types", ADMIN, Some(json!({"code": "store", "name": "Store"})), 201).await;
        let store = json!({"parent_id": root, "unit_type": "store", "name": "Cửa hàng A"});
        let store = json!(id(&expect(&app, "POST", "/api/v1/organizations/units", ADMIN, Some(store), 201).await));
        let variant = product(&app, &store, "CUP-1").await;
        let later = (autoxide_kernel::time::Timestamp::now() + jiff::SignedDuration::from_hours(2)).to_rfc3339();
        let sale = promotion(&root, json!({"kind": "flash_sale", "value_bp": null, "starts_at": later}));
        let sale = id(&expect(&app, "POST", "/api/v1/promotions/promotions", ADMIN, Some(sale), 201).await);
        let path = format!("/api/v1/promotions/promotions/{sale}/participations");
        let offer = json!({"org_id": store, "items": [{"variant_id": variant, "sale_price_minor": 150000, "quantity_limit": 10}]});
        let offered = id(&expect(&app, "POST", &path, ADMIN, Some(offer), 201).await);
        let reason = json!({"reason": "Giá chưa đủ hấp dẫn"});
        expect(&app, "POST", &format!("{path}/{offered}/reject"), ADMIN, Some(reason), 200).await;
    })
    .await
    .unwrap();
}
