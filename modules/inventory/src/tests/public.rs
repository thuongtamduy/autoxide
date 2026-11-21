//! Tình trạng hàng công khai (D-200): không lộ số lớn, đổi ngay sau khi giữ chỗ, ETag → 304,
//! đặt trước hiện `preorder`, combo tính từ thành phần.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::sync::Arc;

use autoxide_catalog::api::SaleMode;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{Emitter, call, drain, emit, raw_event, shop_with, stock_in};
use crate::api::{Inventory, ReserveLine, ReserveRequest};

/// Gọi `GET /public/availability` không đăng nhập; trả trạng thái, `ETag` và thân.
async fn availability(
    app: &TestApp,
    query: &str,
    etag: Option<&str>,
) -> (StatusCode, String, Value) {
    let mut request =
        app.request(Method::GET, &format!("/api/v1/inventory/public/availability?{query}"));
    if let Some(etag) = etag {
        request = request.header("if-none-match", etag);
    }
    let answer = request.send().await.unwrap();
    let status = answer.status();
    let tag =
        answer.headers().get("etag").map(|v| v.to_str().unwrap().to_owned()).unwrap_or_default();
    let body = if answer.body().is_empty() { Value::Null } else { answer.json() };
    (status, tag, body)
}

fn row(body: &Value, variant: Uuid) -> &Value {
    body["data"].as_array().unwrap().iter().find(|row| row["variant_id"] == json!(variant)).unwrap()
}

#[tokio::test]
async fn tinh_trang_cong_khai_khong_lo_so_lon() {
    TestDatabase::run(async |test| {
        let (organizations, outbox) =
            Emitter::new("organizations", &["organizations.public_profile_updated"]);
        let shop = shop_with(test, |builder, _| builder.module(organizations)).await;
        let a = shop.tree.a;
        let store = shop.tree.store.as_uuid();
        // Cửa hàng bán từ kho A.
        let sources = json!({"sources": [
            {"warehouse_org_id": a.as_uuid(), "priority": 1, "active": true},
        ]});
        let (status, _) =
            call(&shop, "PUT", &format!("/sources/{store}"), "boss", Some(sources)).await;
        assert_eq!(status, StatusCode::OK);

        let item = shop.catalog.item();
        let pre = shop.catalog.add(|variant| variant.sale_mode = SaleMode::Preorder);
        let digital = shop.catalog.add(|variant| variant.track_inventory = false);
        let part = shop.catalog.item();
        let kit = shop.catalog.bundle(&[(part, 2)]);
        let hidden = shop.catalog.add(|variant| variant.variant_active = false);
        stock_in(&shop, a, item, 100, None).await;
        stock_in(&shop, a, part, 7, None).await;
        stock_in(&shop, a, hidden, 7, None).await;

        let ids = [item, pre, digital, kit, hidden, Uuid::now_v7()]
            .iter()
            .map(Uuid::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let query = format!("variant_ids={ids}&store_id={store}");
        let (status, etag, body) = availability(&shop, &query, None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(!etag.is_empty());
        // Biến thể ẩn và không có bị bỏ qua.
        assert_eq!(body["data"].as_array().unwrap().len(), 4, "{body}");
        assert_eq!(row(&body, item)["level"], "in_stock");
        assert_eq!(row(&body, item)["quantity"], Value::Null, "không lộ số 100");
        assert_eq!(row(&body, pre)["level"], "preorder");
        assert_eq!(row(&body, digital)["level"], "in_stock");
        // 7 thành phần / 2 mỗi bộ = 3 bộ, dưới ngưỡng 5.
        assert_eq!(row(&body, kit)["level"], "low");
        assert_eq!(row(&body, kit)["quantity"], 3);

        // Không đổi → 304; giữ chỗ 97 → còn 3, mức đổi ngay.
        let (status, _, _) = availability(&shop, &query, Some(&etag)).await;
        assert_eq!(status, StatusCode::NOT_MODIFIED);
        let api: Arc<dyn Inventory> = shop.api::<dyn Inventory>().unwrap();
        let request = ReserveRequest::new(
            Uuid::now_v7(),
            shop.tree.store,
            Timestamp::now() + SignedDuration::from_mins(30),
            vec![ReserveLine::new("1", item, 97)],
        );
        api.reserve(&shop.default_tenant(), &request).await.unwrap();
        let (status, changed, body) = availability(&shop, &query, Some(&etag)).await;
        assert_eq!(status, StatusCode::OK);
        assert_ne!(changed, etag);
        assert_eq!(row(&body, item)["level"], "low");
        assert_eq!(row(&body, item)["quantity"], 3);

        // Bộ lọc "còn hàng ở cửa hàng" của `catalog` tính giống hệt: đặt trước chưa về không tính.
        let tenant = shop.default_tenant();
        let found = api
            .in_stock_at(&tenant, shop.tree.store, &[item, pre, digital, kit, hidden])
            .await
            .unwrap();
        assert_eq!(found, vec![item, digital, kit]);

        // Bỏ `store_id`: mọi cửa hàng nhận hàng tại chỗ (hồ sơ công khai `pickup`).
        let only = format!("variant_ids={item}");
        let (_, _, body) = availability(&shop, &only, None).await;
        assert_eq!(body["data"], json!([]));
        raw_event!(ProfileUpdated, "organizations.public_profile_updated");
        let profile = ProfileUpdated(json!({"after": {
            "public": true, "display_name": "Store", "pickup": true,
            "address": {"country": "VN", "province_code": "79", "commune_code": "26734"},
        }}));
        emit(&shop, &outbox, Some(format!("organizations.unit:{store}")), &profile).await;
        drain(&shop).await;
        let (_, _, body) = availability(&shop, &only, None).await;
        assert_eq!(body["data"].as_array().unwrap().len(), 1, "{body}");
        assert_eq!(body["data"][0]["store_id"], json!(store));

        // Sai đầu vào: 422.
        let (status, _, _) = availability(&shop, "variant_ids=abc", None).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let many = (0..101).map(|_| Uuid::now_v7().to_string()).collect::<Vec<_>>().join(",");
        let (status, _, _) = availability(&shop, &format!("variant_ids={many}"), None).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}
