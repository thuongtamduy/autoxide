//! Sản phẩm, biến thể, combo, đặt trước, rò rỉ giữa khách và giữa hai cửa hàng anh em.

use std::sync::Arc;

use autoxide_kernel::tenancy::DEFAULT_TENANT;
use autoxide_testkit::{TestApp, TestDatabase, branch_leak_check, leak_check, tool};
use axum::http::StatusCode;
use serde_json::json;
use uuid::Uuid;

use super::{ALL, app, call, create_product, product_body};
use crate::api::{Catalog, ProductKind, SaleMode};

#[tokio::test]
async fn tao_sua_mo_ban_luu_tru_va_khong_xoa_duoc_sau_khi_ban() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let (id, _) = create_product(&app, "boss", tree.a, "ao-thun-m", "Áo thun cotton").await;
        let (status, got) = call(&app, "GET", &format!("/products/{id}"), "boss", None).await;
        assert_eq!(status, StatusCode::OK, "{got}");
        assert_eq!(got["data"]["status"], "draft");
        assert_eq!(got["data"]["translations"][0]["slug"], "ao-thun-cotton");
        assert_eq!(got["data"]["variants"][0]["sku"], "ao-thun-m");

        let body = json!({"warranty_months": 12});
        let (status, changed) =
            call(&app, "PATCH", &format!("/products/{id}"), "boss", Some(body)).await;
        assert_eq!((status, &changed["data"]["warranty_months"]), (StatusCode::OK, &json!(12)));

        let status_uri = format!("/products/{id}/status");
        let (status, sold) =
            call(&app, "POST", &status_uri, "boss", Some(json!({"status": "active"}))).await;
        assert_eq!(status, StatusCode::OK, "{sold}");
        assert_eq!(sold["data"]["ever_active"], true);
        // Đang bán → nháp: không được.
        let (status, problem) =
            call(&app, "POST", &status_uri, "boss", Some(json!({"status": "draft"}))).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::CONFLICT, Some("catalog.invalid_status_transition"))
        );
        // Đã từng bán: không xoá được, chỉ lưu trữ.
        let (status, problem) =
            call(&app, "DELETE", &format!("/products/{id}"), "boss", None).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::CONFLICT, Some("catalog.product_not_deletable"))
        );
        let (status, _) =
            call(&app, "POST", &status_uri, "boss", Some(json!({"status": "archived"}))).await;
        assert_eq!(status, StatusCode::OK);

        // Sản phẩm nháp xoá được.
        let (draft, _) = create_product(&app, "boss", tree.a, "nhap", "Nháp").await;
        let (status, _) = call(&app, "DELETE", &format!("/products/{draft}"), "boss", None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _) = call(&app, "GET", &format!("/products/{draft}"), "boss", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    })
    .await
    .unwrap();
}

/// SKU và mã vạch duy nhất trong một khách (không phân biệt hoa thường, khoảng trắng); khách
/// khác dùng cùng SKU được.
#[tokio::test]
async fn sku_va_ma_vach_duy_nhat_trong_khach() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test)
            .tenant("one")
            .tenant("two")
            .user_in("one", "one", 1, &ALL)
            .user_in("two", "two", 2, &ALL)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let body = |sku: &str, barcode: Option<&str>| {
            json!({
                "kind": "physical", "tax_category": "vat.standard", "locale": "vi",
                "text": {"name": format!("Hàng {sku}")},
                "variant": {"sku": sku, "barcode": barcode},
            })
        };
        let (status, created) =
            call(&app, "POST", "/products", "one", Some(body("SKU-1", Some("8934563138165"))))
                .await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let (status, problem) =
            call(&app, "POST", "/products", "one", Some(body(" sku-1 ", None))).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::CONFLICT, Some("catalog.sku_taken"))
        );
        let (status, problem) =
            call(&app, "POST", "/products", "one", Some(body("SKU-2", Some("8934563138165"))))
                .await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::CONFLICT, Some("catalog.barcode_taken"))
        );
        // Mã vạch rỗng là "không có": nhiều biến thể không mã vạch được.
        for sku in ["SKU-3", "SKU-4"] {
            let (status, _) =
                call(&app, "POST", "/products", "one", Some(body(sku, Some("")))).await;
            assert_eq!(status, StatusCode::CREATED);
        }
        // GTIN sai chữ số kiểm tra.
        let (status, problem) =
            call(&app, "POST", "/products", "one", Some(body("SKU-5", Some("8934563138166"))))
                .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        // Khách khác: cùng SKU, cùng mã vạch được.
        let (status, _) =
            call(&app, "POST", "/products", "two", Some(body("SKU-1", Some("8934563138165"))))
                .await;
        assert_eq!(status, StatusCode::CREATED);
    })
    .await
    .unwrap();
}

/// 16 request tạo cùng SKU cùng lúc: đúng một thành công, 15 `catalog.sku_taken`.
#[tokio::test]
async fn sku_dong_thoi_chi_mot_thang() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let app = Arc::new(app);
        let tasks: Vec<_> = (0..16)
            .map(|index| {
                let app = app.clone();
                let body = product_body(tree.a, "RACE", &format!("Đua {index}"));
                tokio::spawn(
                    async move { call(&app, "POST", "/products", "boss", Some(body)).await },
                )
            })
            .collect();
        let mut created = 0;
        for task in tasks {
            let (status, body) = task.await.unwrap();
            if status == StatusCode::CREATED {
                created += 1;
            } else {
                assert_eq!(body["code"], "catalog.sku_taken", "{body}");
            }
        }
        assert_eq!(created, 1);
    })
    .await
    .unwrap();
}

/// Tuỳ chọn: thêm tuỳ chọn thì biến thể cũ nhận giá trị đầu; biến thể theo tổ hợp, trùng tổ hợp
/// → 409; nhãn "Đỏ / M" theo ngôn ngữ qua `dyn Catalog`; bỏ giá trị đang dùng → 409.
#[tokio::test]
async fn to_hop_tuy_chon_duy_nhat_va_nhan_bien_the() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let (id, first) = create_product(&app, "boss", tree.a, "TEE-RED-M", "Áo thun").await;
        let options = json!({"options": [
            {"names": {"vi": "Màu", "en": "Colour"}, "values": [
                {"labels": {"vi": "Đỏ", "en": "Red"}}, {"labels": {"vi": "Xanh", "en": "Blue"}}]},
            {"names": {"vi": "Cỡ"}, "values": [{"labels": {"vi": "M"}}, {"labels": {"vi": "L"}}]},
        ]});
        let uri = format!("/products/{id}/options");
        let (status, product) = call(&app, "PUT", &uri, "boss", Some(options)).await;
        assert_eq!(status, StatusCode::OK, "{product}");
        let red = product["data"]["options"][0]["values"][0]["id"].clone();
        let blue = product["data"]["options"][0]["values"][1]["id"].clone();
        let m = product["data"]["options"][1]["values"][0]["id"].clone();
        let l = product["data"]["options"][1]["values"][1]["id"].clone();
        // Biến thể đầu nhận giá trị đầu của mỗi tuỳ chọn: Đỏ / M.
        let catalog = app.api::<dyn Catalog>().unwrap();
        let tenant = app.default_tenant();
        let first_id: Uuid = first.parse().unwrap();
        let snapshot = catalog.variants(&tenant, &[first_id], Some("en")).await.unwrap();
        assert_eq!(snapshot[0].variant_label, "Red / M");
        let snapshot = catalog.variants(&tenant, &[first_id], None).await.unwrap();
        assert_eq!(snapshot[0].variant_label, "Đỏ / M");

        let variants = format!("/products/{id}/variants");
        let body = json!({"sku": "TEE-BLUE-L", "option_value_ids": [blue, l]});
        let (status, made) = call(&app, "POST", &variants, "boss", Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        let body = json!({"sku": "TEE-RED-M-2", "option_value_ids": [red, m]});
        let (status, problem) = call(&app, "POST", &variants, "boss", Some(body)).await;
        assert_eq!((status, problem["code"].as_str()), (StatusCode::CONFLICT, Some("catalog.option_combination_taken")));
        let body = json!({"sku": "TEE-HALF", "option_value_ids": [red]});
        let (status, _) = call(&app, "POST", &variants, "boss", Some(body)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

        // Bỏ màu Xanh khi biến thể TEE-BLUE-L còn dùng: chặn.
        let fewer = json!({"options": [
            {"id": product["data"]["options"][0]["id"], "names": {"vi": "Màu"}, "values": [{"id": red, "labels": {"vi": "Đỏ"}}]},
            {"id": product["data"]["options"][1]["id"], "names": {"vi": "Cỡ"}, "values": [{"id": m, "labels": {"vi": "M"}}, {"id": l, "labels": {"vi": "L"}}]},
        ]});
        let (status, problem) = call(&app, "PUT", &uri, "boss", Some(fewer)).await;
        assert_eq!((status, problem["code"].as_str()), (StatusCode::CONFLICT, Some("catalog.option_value_in_use")));

        let found = catalog.variant_by_code(&tenant, "tee-blue-l", Some("en")).await.unwrap().unwrap();
        assert_eq!(found.variant_label, "Blue / L");
        assert!(catalog.variant_by_code(&tenant, "nope", None).await.unwrap().is_none());
        // Xoá được biến thể khi còn biến thể khác; biến thể cuối thì không.
        let (status, _) = call(&app, "DELETE", &format!("/variants/{first}"), "boss", None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let last = made["data"]["id"].as_str().unwrap();
        let (status, problem) = call(&app, "DELETE", &format!("/variants/{last}"), "boss", None).await;
        assert_eq!((status, problem["code"].as_str()), (StatusCode::CONFLICT, Some("catalog.last_variant")));
    })
    .await
    .unwrap();
}

/// Combo: chỉ sản phẩm `bundle`; không chứa chính nó hay combo khác; đọc qua `dyn Catalog`.
#[tokio::test]
async fn combo_khong_long_combo() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let mut body = product_body(tree.a, "BOX", "Hộp quà");
        body["kind"] = json!("bundle");
        let (status, made) = call(&app, "POST", "/products", "boss", Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        let bundle = made["data"]["variants"][0]["id"].as_str().unwrap().to_owned();
        let mut other = product_body(tree.a, "BOX-2", "Hộp khác");
        other["kind"] = json!("bundle");
        let (_, other) = call(&app, "POST", "/products", "boss", Some(other)).await;
        let other_bundle = other["data"]["variants"][0]["id"].clone();
        let (_, part) = create_product(&app, "boss", tree.a, "PART", "Thành phần").await;
        let uri = format!("/variants/{bundle}/components");
        let one = |variant: serde_json::Value| json!({"components": [{"variant_id": variant, "quantity": 2}]});
        let (status, saved) = call(&app, "PUT", &uri, "boss", Some(one(json!(part)))).await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        for bad in [json!(bundle), other_bundle] {
            let (status, problem) = call(&app, "PUT", &uri, "boss", Some(one(bad))).await;
            assert_eq!((status, problem["code"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("catalog.bundle_invalid")));
        }
        // Thành phần chỉ gắn được vào combo.
        let (status, problem) =
            call(&app, "PUT", &format!("/variants/{part}/components"), "boss", Some(one(json!(bundle)))).await;
        assert_eq!((status, problem["code"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("catalog.bundle_invalid")));

        let catalog = app.api::<dyn Catalog>().unwrap();
        let tenant = app.default_tenant();
        let bundle_id: Uuid = bundle.parse().unwrap();
        let parts = catalog.bundle_components(&tenant, &[bundle_id]).await.unwrap();
        assert_eq!(parts[&bundle_id][0].quantity, 2);
        let snapshot = catalog.variants(&tenant, &[bundle_id], Some("en")).await.unwrap();
        assert_eq!(snapshot[0].kind, ProductKind::Bundle);
        assert!(!snapshot[0].track_inventory);
        assert!(snapshot[0].requires_shipping);
        assert_eq!((snapshot[0].name.as_str(), snapshot[0].locale.as_str()), ("Hộp quà", "vi"));
    })
    .await
    .unwrap();
}

/// Đặt trước (D-204): ngày có hàng phải sau hôm nay, suất > 0, gom lỗi một lần; ảnh chụp mang
/// `sale_mode` và `allow_backorder` cho bên chưa chuyển.
#[tokio::test]
async fn dat_truoc_phai_co_ngay_sau_hom_nay() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let mut body = product_body(tree.a, "PRE", "Hàng đặt trước");
        body["variant"]["sale_mode"] = json!("preorder");
        body["variant"]["preorder_available_on"] = json!("2020-01-01");
        body["variant"]["preorder_limit"] = json!(0);
        let (status, problem) = call(&app, "POST", "/products", "boss", Some(body.clone())).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        let codes: Vec<&str> = problem["errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["code"].as_str().unwrap())
            .collect();
        assert!(codes.contains(&"catalog.preorder_date_invalid"), "{codes:?}");
        assert!(codes.contains(&"catalog.preorder_limit_invalid"), "{codes:?}");

        let later = (chrono::Utc::now() + chrono::Duration::days(30)).date_naive();
        body["variant"]["preorder_available_on"] = json!(later);
        body["variant"]["preorder_limit"] = json!(100);
        let (status, made) = call(&app, "POST", "/products", "boss", Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        let variant: Uuid = made["data"]["variants"][0]["id"].as_str().unwrap().parse().unwrap();

        // Đổi sang `backorder`: cờ cũ `allow_backorder` đọc được.
        let (status, _) = call(
            &app,
            "PATCH",
            &format!("/variants/{variant}"),
            "boss",
            Some(json!({"sale_mode": "backorder"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let catalog = app.api::<dyn Catalog>().unwrap();
        let snapshot = catalog.variants(&app.default_tenant(), &[variant], None).await.unwrap();
        assert_eq!(snapshot[0].sale_mode, SaleMode::Backorder);
        assert!(snapshot[0].allow_backorder);
        assert_eq!(snapshot[0].preorder_available_on, None);
    })
    .await
    .unwrap();
}

/// REST và MCP gọi cùng service; người chỉ đọc không thấy tool ghi.
#[tokio::test]
async fn rest_va_mcp_dung_chung_service() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let client = app.mcp("boss").await.unwrap();
        let made = client
            .call_tool(tool("catalog.create_product", product_body(tree.a, "MCP-1", "Qua MCP")))
            .await
            .unwrap();
        assert_eq!(made.is_error, Some(false), "{made:?}");
        let id = made.structured_content.unwrap()["data"]["id"].as_str().unwrap().to_owned();
        let (status, got) = call(&app, "GET", &format!("/products/{id}"), "boss", None).await;
        assert_eq!(
            (status, got["data"]["translations"][0]["name"].as_str()),
            (StatusCode::OK, Some("Qua MCP"))
        );
        let listed = client.call_tool(tool("catalog.list_products", json!({"q": "qua mcp"}))).await;
        let listed = listed.unwrap().structured_content.unwrap();
        assert_eq!(listed["data"][0]["id"], id.as_str());
        let taken = client
            .call_tool(tool("catalog.create_product", product_body(tree.a, "mcp-1", "Trùng")))
            .await
            .unwrap();
        assert_eq!(taken.structured_content.unwrap()["code"], "catalog.sku_taken");
        let removed =
            client.call_tool(tool("catalog.delete_product", json!({"id": id}))).await.unwrap();
        assert_eq!(removed.is_error, Some(false));
        client.cancel().await.unwrap();
    })
    .await
    .unwrap();
}

/// Rò rỉ: khách khác không thấy gì; nhân viên cửa hàng B (độ sâu `unit`) không thấy / sửa sản
/// phẩm, bảng giá của cửa hàng A.
#[tokio::test]
async fn ro_ri_giua_hai_cua_hang_anh_em_va_giua_khach() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let marker = "MARKER-STORE-A";
        let (product, variant) = create_product(&app, "store_a", tree.a, "A-1", marker).await;
        let list = json!({"org_id": tree.a.as_uuid(), "name": marker, "currency": "VND", "tax_inclusive": true});
        let (status, list) = call(&app, "POST", "/price-lists", "store_a", Some(list)).await;
        assert_eq!(status, StatusCode::CREATED, "{list}");
        let list_id = list["data"]["id"].as_str().unwrap().to_owned();
        let ids = [("id", product.clone()), ("id", variant.clone()), ("id", list_id.clone())];

        let leaks = branch_leak_check(&app, "store_b", marker, &ids).await.unwrap();
        assert_eq!(leaks, Vec::<String>::new());
        let (status, _) = call(
            &app,
            "PATCH",
            &format!("/products/{product}"),
            "store_b",
            Some(json!({"warranty_months": 1})),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = call(
            &app,
            "PATCH",
            &format!("/price-lists/{list_id}"),
            "store_b",
            Some(json!({"name": "x"})),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        // Tạo ở cửa hàng A bằng người của B: ngoài phạm vi.
        let (status, problem) =
            call(&app, "POST", "/products", "store_b", Some(product_body(tree.a, "B-1", "x"))).await;
        assert_eq!((status, problem["code"].as_str()), (StatusCode::FORBIDDEN, Some("org_out_of_scope")));
        let _ = DEFAULT_TENANT;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ro_ri_giua_khach() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test)
            .tenant("one")
            .tenant_dedicated("two")
            .user_in("one", "one", 1, &ALL)
            .user_in("two", "two", 1, &ALL)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let marker = "MARKER-TENANT-ONE";
        let body = json!({
            "kind": "physical", "tax_category": "vat.standard", "locale": "vi",
            "text": {"name": marker}, "variant": {"sku": "ONE-1"},
        });
        let (status, made) = call(&app, "POST", "/products", "one", Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        let product = made["data"]["id"].as_str().unwrap().to_owned();
        let variant = made["data"]["variants"][0]["id"].as_str().unwrap().to_owned();
        let leaks = leak_check(&app, "two", marker, &[("id", product.clone()), ("id", variant)])
            .await
            .unwrap();
        assert_eq!(leaks, Vec::<String>::new());
        let (status, _) = call(&app, "DELETE", &format!("/products/{product}"), "two", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    })
    .await
    .unwrap();
}

/// Hai lần đổi ngày đặt trước cùng lúc sang cùng một ngày: chỉ một event "ngày đã đổi" (cùng mẫu
/// lỗi với storefront, #246). Postgres từng cho cả hai cùng đọc ngày cũ và cùng phát event.
#[tokio::test]
async fn doi_ngay_dat_truoc_dong_thoi_chi_phat_mot_event() {
    use sea_orm::{ConnectionTrait as _, Statement};
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let mut body = product_body(tree.a, "PRE-R", "Hàng đặt trước");
        let later = (chrono::Utc::now() + chrono::Duration::days(30)).date_naive();
        body["variant"]["sale_mode"] = json!("preorder");
        body["variant"]["preorder_available_on"] = json!(later);
        body["variant"]["preorder_limit"] = json!(100);
        let (status, made) = call(&app, "POST", "/products", "boss", Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        let variant = made["data"]["variants"][0]["id"].as_str().unwrap().to_owned();
        let uri = format!("/variants/{variant}");
        let moved = json!({"preorder_available_on": later + chrono::Duration::days(7)});
        let (first, second) = tokio::join!(
            call(&app, "PATCH", &uri, "boss", Some(moved.clone())),
            call(&app, "PATCH", &uri, "boss", Some(moved.clone())),
        );
        assert_eq!((first.0, second.0), (StatusCode::OK, StatusCode::OK), "{first:?} {second:?}");
        let read = app.database().read();
        let sql = "SELECT id FROM catalog_events WHERE name = 'catalog.preorder_date_changed'";
        let rows = read
            .query_all_raw(Statement::from_string(read.get_database_backend(), sql))
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
    })
    .await
    .unwrap();
}
