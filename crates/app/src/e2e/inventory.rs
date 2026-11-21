//! Test đầu-cuối của module `inventory` trên chương trình thật (bước 42): biến thể là của module
//! `catalog` thật, tồn đầu kỳ nhập bằng điều chỉnh có giá vốn qua REST, đọc lại tồn và sổ. Chạy
//! trên cả SQLite và Postgres; giữ hàng qua `dyn Inventory` như `orders` sẽ gọi.

use autoxide_inventory::api::{Inventory, ReserveLine, ReserveRequest};
use autoxide_kernel::tenancy::{DEFAULT_TENANT, TenantContext, root_of};
use autoxide_kernel::time::Timestamp;
use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use jiff::SignedDuration;
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

        // Giữ hàng như `orders` (bước 45) sẽ giữ: qua `dyn Inventory`, biến thể đọc từ `catalog` thật.
        let inventory = runtime.modules.api::<dyn Inventory>().unwrap();
        let tenant = TenantContext::default_on(runtime.database.clone());
        let variant_id: Uuid = variant.parse().unwrap();
        let request = ReserveRequest::new(
            Uuid::now_v7(),
            root_of(DEFAULT_TENANT),
            Timestamp::now() + SignedDuration::from_mins(15),
            vec![ReserveLine::new("1", variant_id, 5)],
        );
        let held = inventory.reserve(&tenant, &request).await.unwrap().into_inner();
        assert_eq!(held.lines[0].warehouse_org_id, root_of(DEFAULT_TENANT));
        let (_, _, levels) = call(&runtime, "GET", &uri, &headers, Vec::new()).await;
        assert_eq!(levels["data"][0]["available"], 7);
        assert_eq!(levels["data"][0]["reserved"], 5);
    })
    .await
    .unwrap();
}
