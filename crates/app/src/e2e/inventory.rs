//! Test đầu-cuối của module `inventory` trên chương trình thật (bước 42): biến thể là của module
//! `catalog` thật, tồn đầu kỳ nhập bằng điều chỉnh có giá vốn qua REST, đọc lại tồn và sổ. Chạy
//! trên cả SQLite và Postgres.

use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use serde_json::json;
use uuid::Uuid;

use crate::e2e::media::{admin_token, call, start};

#[tokio::test]
async fn ton_dau_ky_cho_bien_the_cua_catalog_that() {
    TestDatabase::run(async |test| {
        let dir = std::env::temp_dir().join(format!("autoxide-inventory-{}", Uuid::now_v7()));
        let storage = [("AUTOXIDE_FILES_DIR", dir.display().to_string())];
        let runtime = start(test.url(), &storage).await;
        let token = admin_token(&runtime, "inventory@example.com").await;
        let auth = format!("Bearer {token}");
        let headers = [("authorization", auth.as_str()), ("content-type", "application/json")];
        let post = |uri: &'static str, body: serde_json::Value| {
            let runtime = &runtime;
            let headers = &headers;
            async move { call(runtime, "POST", uri, headers, body.to_string().into_bytes()).await }
        };

        let product = json!({
            "kind": "physical", "tax_category": "vat.standard", "locale": "vi",
            "text": {"name": "Ấm trà"}, "variant": {"sku": "TEA-1", "weight_grams": 500},
        });
        let (status, _, created) = post("/api/v1/catalog/products", product).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let variant = created["data"]["variants"][0]["id"].as_str().unwrap().to_owned();

        // Tồn đầu kỳ ở nút gốc (cửa hàng đơn lẻ), có giá vốn.
        let adjustment = json!({
            "org_id": created["data"]["org_id"],
            "reference": "opening-1",
            "lines": [{"variant_id": variant, "quantity": 12, "reason_code": "found",
                       "unit_cost": {"amount": "85000", "currency": "VND"}}],
        });
        let (status, _, made) = post("/api/v1/inventory/adjustments", adjustment).await;
        assert_eq!(status, StatusCode::CREATED, "{made}");

        let uri = format!("/api/v1/inventory/levels?variant_id={variant}");
        let (status, _, levels) = call(&runtime, "GET", &uri, &headers, Vec::new()).await;
        assert_eq!(status, StatusCode::OK, "{levels}");
        let level = &levels["data"][0];
        assert_eq!((level["on_hand"].as_i64(), level["available"].as_i64()), (Some(12), Some(12)));
        assert_eq!(level["average_cost"], json!({"amount": "85000", "currency": "VND"}));
    })
    .await
    .unwrap();
}
