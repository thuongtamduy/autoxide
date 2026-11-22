//! Khối bán hàng của `catalog` (bước 67): thẻ sản phẩm, lưới (chọn tay và theo bộ lọc), bộ sưu
//! tập — dữ liệu như đọc công khai, sản phẩm không còn hiện thì `None`, một lần gọi cho mọi khối
//! cùng loại. Chạy trên cả SQLite và Postgres.

use autoxide_kernel::blocks::{BlockRef, BlockRequest};
use autoxide_kernel::tenancy::Site;
use autoxide_testkit::TestDatabase;
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

use super::public::{drain, on_sale, price_list};
use super::{app, call};

fn names(products: &Value) -> Vec<&str> {
    products.as_array().unwrap().iter().map(|card| card["name"].as_str().unwrap()).collect()
}

#[tokio::test]
async fn khoi_ban_hang_doc_nhu_trang_cong_khai() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let list = price_list(&app, tree.chain).await;
        let (p1, _) = on_sale(&app, &list, tree.chain, "B-1", "Một", 100_000).await;
        let (p2, _) = on_sale(&app, &list, tree.chain, "B-2", "Hai", 200_000).await;
        drain(&app).await;
        let collection =
            json!({"kind": "manual", "status": "active", "texts": {"vi": {"title": "Tuyển chọn"}}});
        let (status, made) = call(&app, "POST", "/collections", "boss", Some(collection)).await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        let collection = made["data"]["id"].as_str().unwrap().to_owned();
        let items = app
            .request(Method::PUT, &format!("/api/v1/catalog/collections/{collection}/items"))
            .token("boss")
            .header("if-match", "*")
            .json(json!({"product_ids": [p2, p1]}))
            .send()
            .await
            .unwrap();
        assert_eq!(items.status(), StatusCode::OK, "{}", items.json());

        // Khối có trong sổ, kèm schema.
        let listed = app.get("/api/v1/kernel/blocks").token("boss").send().await.unwrap().json();
        let blocks: Vec<&str> = listed["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["name"].as_str().unwrap())
            .collect();
        for name in ["catalog.collection", "catalog.product_card", "catalog.product_grid"] {
            assert!(blocks.contains(&name), "{name} in {blocks:?}");
        }

        let registry = app.modules().blocks();
        let tenant = app.default_tenant();
        let site = Site::of(&tenant, tree.chain).await.unwrap();
        let request = BlockRequest::new(&tenant, &site, "vi");
        let missing = "0190a1b2-0000-7000-8000-000000000099";
        let items = [
            ("catalog.product_card", BlockRef::new("card", json!({"product_id": p1}))),
            ("catalog.product_card", BlockRef::new("gone", json!({"product_id": missing}))),
            (
                "catalog.product_grid",
                BlockRef::new("picked", json!({"product_ids": [p2, missing, p1]})),
            ),
            (
                "catalog.product_grid",
                BlockRef::new("cheap", json!({"sort": "price_asc", "limit": 1})),
            ),
            ("catalog.collection", BlockRef::new("set", json!({"collection_id": collection}))),
            ("catalog.collection", BlockRef::new("no_set", json!({"collection_id": missing}))),
        ];
        let data = registry.resolve(&request, &items).await;
        assert_eq!(data["card"]["product"]["name"], "Một");
        assert_eq!(data["card"]["product"]["price_from"]["amount"], "100000");
        assert_eq!(data["gone"], Value::Null);
        assert_eq!(names(&data["picked"]["products"]), ["Hai", "Một"]);
        assert_eq!(names(&data["cheap"]["products"]), ["Một"]);
        assert_eq!(data["set"]["collection"]["title"], "Tuyển chọn");
        assert_eq!(names(&data["set"]["products"]), ["Hai", "Một"]);
        assert_eq!(data["no_set"], Value::Null);

        // Ngôn ngữ catalog không có: khối không có dữ liệu, trang vẫn trả.
        let request = BlockRequest::new(&tenant, &site, "xx");
        assert_eq!(registry.resolve(&request, &items[..1]).await["card"], Value::Null);
    })
    .await
    .unwrap();
}
