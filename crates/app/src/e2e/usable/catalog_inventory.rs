//! Danh mục, kho và cách giao dùng được như người bán thật (cả SQLite và Postgres): lịch giao, vùng,
//! cách giao theo bảng cân; sản phẩm, giá, bản dịch, thuộc tính, bộ sưu tập, giá sàn, nhập CSV; tồn,
//! nhà cung cấp, đơn mua, nhận hàng, kiểm kê, chuyển kho, trả nhà cung cấp.

use autoxide_kernel::tenancy::{DEFAULT_TENANT, root_of};
use autoxide_testkit::TestDatabase;
use serde_json::{Value, json};

use super::kit::{ADMIN, app, call_tool, drain, expect, get, id, import_reference_data};

fn vnd(amount: i64) -> Value {
    json!({"amount": amount.to_string(), "currency": "VND"})
}

#[tokio::test]
async fn lich_giao_vung_va_cach_giao() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        import_reference_data(&app).await;
        let root = json!(root_of(DEFAULT_TENANT));

        let calendar = json!({"org_id": root, "name": "Kho chính", "time_zone": "Asia/Ho_Chi_Minh"});
        let calendar = id(&expect(&app, "POST", "/api/v1/shipping/calendars", ADMIN, Some(calendar), 201).await);
        let listed = get(&app, "/api/v1/shipping/calendars").await;
        assert_eq!(listed["data"].as_array().unwrap().len(), 1, "{listed}");
        let path = format!("/api/v1/shipping/calendars/{calendar}");
        expect(&app, "PATCH", &path, ADMIN, Some(json!({"name": "Kho Hà Nội"})), 200).await;
        let day = format!("{path}/days/2026-12-31");
        expect(&app, "PUT", &day, ADMIN, Some(json!({"kind": "closed", "note": "Nghỉ"})), 200).await;
        expect(&app, "DELETE", &day, ADMIN, None, 200).await;

        let zone = json!({"org_id": root, "name": "Toàn quốc", "regions": [{"country": "VN"}]});
        let zone = id(&expect(&app, "POST", "/api/v1/shipping/zones", ADMIN, Some(zone), 201).await);
        let zone_path = format!("/api/v1/shipping/zones/{zone}");
        get(&app, &zone_path).await;
        expect(&app, "PATCH", &zone_path, ADMIN, Some(json!({"name": "Cả nước"})), 200).await;
        let regions = json!({"regions": [{"country": "VN", "province_code": "01"}, {"country": "VN", "province_code": "79"}]});
        expect(&app, "PUT", &format!("{zone_path}/regions"), ADMIN, Some(regions), 200).await;

        let method = json!({"org_id": root, "code": "standard", "kind": "table_rate", "currency": "VND",
                            "names": {"vi": "Giao tiêu chuẩn", "en": "Standard"}, "calendar_id": calendar});
        let method = id(&expect(&app, "POST", "/api/v1/shipping/methods", ADMIN, Some(method), 201).await);
        let method_path = format!("/api/v1/shipping/methods/{method}");
        let rates = json!({"rates": [{"zone_id": zone, "min_weight_grams": 0, "amount": vnd(30_000)}]});
        expect(&app, "PUT", &format!("{method_path}/rates"), ADMIN, Some(rates), 200).await;
        let got = get(&app, &method_path).await;
        assert_eq!(got["data"]["code"], "standard", "{got}");
        expect(&app, "DELETE", &method_path, ADMIN, None, 204).await;
        expect(&app, "DELETE", &zone_path, ADMIN, None, 204).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn san_pham_gia_ban_dich_thuoc_tinh_bo_suu_tap_va_nhap_csv() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let product = json!({"kind": "physical", "tax_category": "vat.standard", "locale": "vi",
            "text": {"name": "Ấm trà"}, "variant": {"sku": "TEA-1", "barcode": "8930000000019", "weight_grams": 600}});
        let created = expect(&app, "POST", "/api/v1/catalog/products", ADMIN, Some(product), 201).await;
        let product = id(&created);
        let variant = created["data"]["variants"][0]["id"].as_str().unwrap().to_owned();

        let found = get(&app, "/api/v1/catalog/variants?code=8930000000019").await;
        assert_eq!(found["data"][0]["id"], variant.as_str(), "{found}");
        call_tool(&app, ADMIN, "catalog.get_product", json!({"id": product})).await;
        call_tool(&app, ADMIN, "catalog.update_product", json!({"id": product, "warranty_months": 12})).await;
        call_tool(
            &app,
            ADMIN,
            "catalog.upsert_variant",
            json!({"id": variant, "update": {"weight_grams": 650}}),
        )
        .await;

        let list = json!({"name": "Bán lẻ", "currency": "VND", "tax_inclusive": true});
        let list = id(&expect(&app, "POST", "/api/v1/catalog/price-lists", ADMIN, Some(list), 201).await);
        let prices = json!({"price_list_id": list, "reference": "opening",
                            "prices": [{"variant_id": variant, "amount": "150000"}]});
        call_tool(&app, ADMIN, "catalog.add_prices", prices).await;
        call_tool(&app, ADMIN, "catalog.set_product_status", json!({"id": product, "status": "active"})).await;

        let translation = format!("/api/v1/catalog/products/{product}/translations/en");
        expect(&app, "PUT", &translation, ADMIN, Some(json!({"name": "Teapot"})), 200).await;
        expect(&app, "DELETE", &translation, ADMIN, None, 200).await;

        let attribute = json!({"code": "brand", "data_type": "text", "names": {"vi": "Thương hiệu"}});
        let attribute = id(&expect(&app, "POST", "/api/v1/catalog/attributes", ADMIN, Some(attribute), 201).await);
        let names = json!({"names": {"vi": "Thương hiệu", "en": "Brand"}});
        expect(&app, "PATCH", &format!("/api/v1/catalog/attributes/{attribute}"), ADMIN, Some(names), 200).await;

        let collection = json!({"kind": "manual", "status": "active", "texts": {"vi": {"title": "Tuyển chọn"}}});
        let collection = id(&expect(&app, "POST", "/api/v1/catalog/collections", ADMIN, Some(collection), 201).await);
        let collection = format!("/api/v1/catalog/collections/{collection}");
        get(&app, &collection).await;
        get(&app, &format!("{collection}/items")).await;
        expect(&app, "DELETE", &collection, ADMIN, None, 204).await;

        let guard = json!({"action": "warn", "currency": "VND", "min_price": vnd(10_000)});
        let guard = id(&expect(&app, "POST", "/api/v1/catalog/price-guards", ADMIN, Some(guard), 201).await);
        expect(&app, "DELETE", &format!("/api/v1/catalog/price-guards/{guard}"), ADMIN, None, 204).await;

        let csv = "sku,name,tax_category\r\nPOT-1,Nồi,vat.standard\r\n";
        let started = call_tool(&app, ADMIN, "catalog.start_import", json!({"csv": csv})).await;
        let import = started["id"].as_str().unwrap_or_else(|| panic!("{started}")).to_owned();
        drain(&app).await;
        call_tool(&app, ADMIN, "catalog.import_status", json!({"id": import})).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ton_kho_nha_cung_cap_don_mua_kiem_ke_chuyen_kho_va_tra_hang() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let root = json!(root_of(DEFAULT_TENANT));
        let product = json!({"kind": "physical", "tax_category": "vat.standard", "locale": "vi",
            "text": {"name": "Ấm trà"}, "variant": {"sku": "TEA-1", "weight_grams": 600}});
        let created = expect(&app, "POST", "/api/v1/catalog/products", ADMIN, Some(product), 201).await;
        let variant = created["data"]["variants"][0]["id"].as_str().unwrap().to_owned();

        let opening = json!({"org_id": root, "reference": "opening-1", "lines": [{"variant_id": variant,
            "quantity": 20, "reason_code": "found", "unit_cost": vnd(85_000)}]});
        expect(&app, "POST", "/api/v1/inventory/adjustments", ADMIN, Some(opening), 201).await;
        let levels = get(&app, &format!("/api/v1/inventory/levels?variant_id={variant}")).await;
        let level = levels["data"][0]["id"].as_str().unwrap_or_else(|| panic!("{levels}")).to_owned();
        get(&app, &format!("/api/v1/inventory/levels/{level}")).await;

        let supplier = json!({"code": "TRA-VIET", "name": "Trà Việt", "currency": "VND"});
        let supplier = id(&expect(&app, "POST", "/api/v1/inventory/suppliers", ADMIN, Some(supplier), 201).await);
        let path = format!("/api/v1/inventory/suppliers/{supplier}");
        expect(&app, "PATCH", &path, ADMIN, Some(json!({"payment_terms_days": 30})), 200).await;

        let lines = json!([{"variant_id": variant, "quantity": 5, "unit_cost": vnd(80_000)}]);
        let order = json!({"org_id": root, "supplier_id": supplier, "lines": lines});
        let order = id(&expect(&app, "POST", "/api/v1/inventory/purchase-orders", ADMIN, Some(order), 201).await);
        let path = format!("/api/v1/inventory/purchase-orders/{order}");
        let changed = json!({"org_id": root, "supplier_id": supplier, "note": "Giao trước Tết",
            "lines": [{"variant_id": variant, "quantity": 8, "unit_cost": vnd(80_000)}]});
        expect(&app, "PATCH", &path, ADMIN, Some(changed), 200).await;
        expect(&app, "POST", &format!("{path}/cancel"), ADMIN, None, 200).await;

        let receipt = json!({"org_id": root, "supplier_id": supplier, "reference": "GRN-1", "lines": [
            {"variant_id": variant, "quantity": 4, "unit_cost_amount": "82000", "unit_cost_currency": "VND"}]});
        call_tool(&app, ADMIN, "inventory.create_receipt", receipt).await;

        let count = id(&expect(&app, "POST", "/api/v1/inventory/counts", ADMIN, Some(json!({"org_id": root})), 201).await);
        let path = format!("/api/v1/inventory/counts/{count}");
        get(&app, &path).await;
        expect(&app, "POST", &format!("{path}/cancel"), ADMIN, None, 200).await;

        // Kho thứ hai để chuyển hàng.
        let kind = json!({"code": "warehouse", "name": "Kho"});
        expect(&app, "POST", "/api/v1/organizations/unit-types", ADMIN, Some(kind), 201).await;
        let unit = json!({"parent_id": root, "unit_type": "warehouse", "name": "Kho Sài Gòn"});
        let second = id(&expect(&app, "POST", "/api/v1/organizations/units", ADMIN, Some(unit), 201).await);
        let transfer = json!({"from_org_id": root, "to_org_id": second, "lines": [{"variant_id": variant, "quantity": 2}]});
        let transfer = id(&expect(&app, "POST", "/api/v1/inventory/transfers", ADMIN, Some(transfer), 201).await);
        get(&app, &format!("/api/v1/inventory/transfers/{transfer}")).await;

        let back = json!({"org_id": root, "supplier_id": supplier, "reference": "RTV-1",
                          "lines": [{"variant_id": variant, "quantity": 1}]});
        let back = id(&expect(&app, "POST", "/api/v1/inventory/supplier-returns", ADMIN, Some(back), 201).await);
        get(&app, &format!("/api/v1/inventory/supplier-returns/{back}")).await;
    })
    .await
    .unwrap();
}
