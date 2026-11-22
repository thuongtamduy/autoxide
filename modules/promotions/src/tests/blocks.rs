//! Khối khuyến mại (bước 67): flash sale có giá và suất còn thật, ô lưu voucher chỉ cho chương
//! trình công khai cho lưu; chương trình của website khác, không công khai hay không cho lưu thì
//! khối ẩn (`null`). Chạy trên cả SQLite và Postgres.

use super::CatalogExt as _;
use autoxide_kernel::blocks::{BlockRef, BlockRequest};
use autoxide_kernel::tenancy::Site;
use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use serde_json::{Value, json};

use super::{activate, body, call, create, running, setup};

#[tokio::test]
async fn khoi_flash_sale_va_luu_voucher() {
    TestDatabase::run(async |test| {
        let setup = setup(test, true).await;
        let variant = setup.catalog.add(setup.tree.a, 200_000, &[]);
        let sale = create(
            &setup,
            body(setup.tree.a, json!({"kind": "flash_sale", "value_bp": null, "public": true})),
        )
        .await;
        let items = json!({"items": [{"variant_id": variant, "sale_price_minor": 120000, "quantity_limit": 5}]});
        let (status, _) =
            call(&setup.app, "PUT", &format!("/promotions/{sale}/flash-items"), "boss", Some(items)).await;
        assert_eq!(status, StatusCode::OK);
        let (status, activated) = activate(&setup, &sale).await;
        assert_eq!(status, StatusCode::OK, "{activated}");
        let voucher = running(
            &setup,
            body(setup.tree.a, json!({"trigger": "voucher", "claimable": true, "public": true, "per_user_claim_limit": 1})),
        )
        .await;
        let plain = running(&setup, body(setup.tree.a, json!({"public": true}))).await;
        let other_site = running(
            &setup,
            body(setup.tree.b, json!({"trigger": "voucher", "claimable": true, "public": true})),
        )
        .await;

        let blocks = setup.app.modules().blocks();
        let tenant = setup.tenant();
        let site = Site::of(&tenant, setup.tree.a).await.unwrap();
        let request = BlockRequest::new(&tenant, &site, "vi");
        let refs = [
            ("promotions.flash_sale", BlockRef::new("sale", json!({"promotion_id": sale}))),
            ("promotions.flash_sale", BlockRef::new("not_sale", json!({"promotion_id": plain}))),
            ("promotions.voucher_claim", BlockRef::new("voucher", json!({"promotion_id": voucher}))),
            ("promotions.voucher_claim", BlockRef::new("plain", json!({"promotion_id": plain}))),
            ("promotions.voucher_claim", BlockRef::new("elsewhere", json!({"promotion_id": other_site}))),
        ];
        let data = blocks.resolve(&request, &refs).await;
        assert_eq!(data["sale"]["state"], "running");
        assert_eq!(data["sale"]["items"][0]["sale_price_minor"], 120_000);
        assert_eq!(data["sale"]["items"][0]["quantity_left"], 5);
        assert_eq!(data["not_sale"], Value::Null);
        assert_eq!(data["voucher"]["claimable"], true);
        assert_eq!(data["voucher"]["title"], "Giảm giá");
        assert_eq!(data["plain"], Value::Null, "not claimable");
        assert_eq!(data["elsewhere"], Value::Null, "another website");
    })
    .await
    .unwrap();
}
