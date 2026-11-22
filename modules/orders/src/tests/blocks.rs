//! Khối của `orders` (bước 67): bốn khối có trong sổ; khối đánh giá trả số sao và đánh giá đã đăng
//! như endpoint công khai, sản phẩm chưa có đánh giá vẫn có dữ liệu (đếm 0). Chạy trên cả SQLite
//! và Postgres.

use autoxide_kernel::blocks::{BlockRef, BlockRequest};
use autoxide_kernel::tenancy::Site;
use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use serde_json::json;

use super::returns::delivered_order;
use super::{Call, shop};

#[tokio::test]
async fn khoi_danh_gia_va_nut_mua() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let blocks = shop.app.modules().blocks();
        for name in ["orders.add_to_cart", "orders.buy_now", "orders.cart_link", "orders.reviews"] {
            assert!(blocks.get(name).is_some(), "{name}");
        }
        assert!(
            blocks
                .check_props("orders.add_to_cart", &json!({"quantity": 1}), "p")
                .iter()
                .any(|error| error.field == "p.product_id" && error.code == "required")
        );

        let (order, line) = delivered_order(&shop).await;
        let (status, created) = Call::as_user(&shop, "buyer1")
            .send(
                "POST",
                &format!("/me/orders/{order}/lines/{line}/review"),
                Some(json!({"rating": 4, "body": "Vải mát"})),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let product = created["data"]["product_id"].as_str().unwrap().to_owned();

        let tenant = shop.app.default_tenant();
        let site = Site::whole(&tenant);
        let request = BlockRequest::new(&tenant, &site, "vi");
        let refs = [
            ("orders.reviews", BlockRef::new("r", json!({"product_id": product, "limit": 5}))),
            ("orders.reviews", BlockRef::new("none", json!({"product_id": uuid::Uuid::now_v7()}))),
        ];
        let data = blocks.resolve(&request, &refs).await;
        assert_eq!(data["r"]["stats"]["count"], 1);
        assert_eq!(data["r"]["stats"]["average"], "4.0");
        assert_eq!(data["r"]["reviews"][0]["body"], "Vải mát");
        assert_eq!(data["r"]["reviews"][0]["verified_buyer"], true);
        assert_eq!(data["none"]["stats"]["count"], 0);
        assert_eq!(data["none"]["reviews"], json!([]));
    })
    .await
    .unwrap();
}
