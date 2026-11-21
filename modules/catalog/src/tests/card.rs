//! Thẻ sản phẩm và dữ liệu cho JSON-LD (D-201): tóm tắt biến thể, thương hiệu, đơn giá theo đơn
//! vị cơ sở, URL từng biến thể, giá ngay trước giá giảm tạm thời.

use autoxide_kernel::time::Timestamp;
use axum::http::StatusCode;
use jiff::SignedDuration;
use serde_json::json;

use super::public::{drain, on_sale, price_list, public};
use super::{app, call};

fn vnd(amount: i64) -> serde_json::Value {
    json!({"amount": amount.to_string(), "currency": "VND"})
}

#[tokio::test]
async fn the_san_pham_co_tom_tat_thuong_hieu_va_don_gia() {
    autoxide_testkit::TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let list = price_list(&app, tree.chain).await;
        let (id, first) = on_sale(&app, &list, tree.chain, "SUA-1", "Sữa tươi", 30_000).await;

        // Hai cỡ: 1 l (biến thể đầu) và 500 ml (rẻ hơn, đơn giá cao hơn).
        let options = json!({"options": [{"names": {"vi": "Cỡ", "en": "Size"}, "values": [
            {"labels": {"vi": "1 lít"}}, {"labels": {"vi": "500 ml"}}]}]});
        let (status, product) =
            call(&app, "PUT", &format!("/products/{id}/options"), "boss", Some(options)).await;
        assert_eq!(status, StatusCode::OK, "{product}");
        let half = product["data"]["options"][0]["values"][1]["id"].clone();
        let body = json!({"net_quantity": 1, "net_unit": "l"});
        let (status, changed) =
            call(&app, "PATCH", &format!("/variants/{first}"), "boss", Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{changed}");
        assert_eq!(changed["data"]["net_unit"], "l");
        let body = json!({"sku": "SUA-500", "option_value_ids": [half], "net_quantity": 500, "net_unit": "ml"});
        let (status, made) =
            call(&app, "POST", &format!("/products/{id}/variants"), "boss", Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        let small = made["data"]["id"].as_str().unwrap().to_owned();
        let body = json!({"reference": "small", "prices": [{"variant_id": small, "amount": vnd(17_000)}]});
        let (status, _) =
            call(&app, "POST", &format!("/price-lists/{list}/prices"), "boss", Some(body)).await;
        assert_eq!(status, StatusCode::OK);
        // Lượng tịnh phải đi đôi với đơn vị.
        let body = json!({"net_quantity": 2, "net_unit": null});
        let (status, _) = call(&app, "PATCH", &format!("/variants/{small}"), "boss", Some(body)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

        // Thương hiệu là thuộc tính mã `brand`.
        let body = json!({"code": "brand", "data_type": "text", "names": {"vi": "Thương hiệu"}});
        let (_, made) = call(&app, "POST", "/attributes", "boss", Some(body)).await;
        let brand = made["data"]["id"].clone();
        let body = json!({"attributes": [{"attribute_id": brand, "value": "Đồng Quê"}]});
        let (status, _) =
            call(&app, "PUT", &format!("/products/{id}/attributes"), "boss", Some(body)).await;
        assert_eq!(status, StatusCode::OK);
        drain(&app).await;

        let site = tree.chain.as_uuid();
        let (_, listed, _) = public(&app, &format!("/products?locale=vi&site={site}"), None).await;
        let card = &listed["data"][0];
        assert_eq!(card["variant_summary"], json!([{"name": "Cỡ", "count": 2}]));
        assert_eq!(card["brand"], "Đồng Quê");
        // Rẻ nhất là 500 ml giá 17 000 → 34 000 / l.
        assert_eq!(card["price_from"], vnd(17_000));
        assert_eq!(card["unit_price"], json!({"price": vnd(34_000), "per": "l"}));

        let (_, page, _) =
            public(&app, &format!("/products/by-slug/sua-tuoi?locale=vi&site={site}"), None).await;
        let variants = page["data"]["variants"].as_array().unwrap();
        let one_litre = variants.iter().find(|item| item["sku"] == "SUA-1").unwrap();
        assert_eq!(one_litre["unit_price"], json!({"price": vnd(30_000), "per": "l"}));
        assert_eq!(one_litre["path"], format!("/products/sua-tuoi?variant={first}"));
        assert!(one_litre["regular_price"].is_null());
    })
    .await
    .unwrap();
}

/// Giá giảm tạm thời: trang sản phẩm có `regular_price` (giá ngay trước) và `price_valid_until`.
#[tokio::test]
async fn gia_truoc_giam_tam_thoi_cho_json_ld() {
    autoxide_testkit::TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let list = price_list(&app, tree.chain).await;
        let (_, variant) = on_sale(&app, &list, tree.chain, "B-1", "Bình nước", 200_000).await;
        // Giá giảm bắt đầu ngay (sau dòng giá gốc), hết sau ba ngày.
        let end = Timestamp::now() + SignedDuration::from_hours(24 * 3);
        let body = json!({"reference": "sale", "prices": [
            {"variant_id": variant, "amount": vnd(150_000), "valid_to": end}]});
        let (status, _) =
            call(&app, "POST", &format!("/price-lists/{list}/prices"), "boss", Some(body)).await;
        assert_eq!(status, StatusCode::OK);
        drain(&app).await;
        let site = tree.chain.as_uuid();
        let (_, page, _) =
            public(&app, &format!("/products/by-slug/binh-nuoc?locale=vi&site={site}"), None).await;
        let offer = &page["data"]["variants"][0];
        assert_eq!(offer["price"], vnd(150_000));
        assert_eq!(offer["regular_price"], vnd(200_000));
        assert!(offer["price_valid_until"].is_string());
    })
    .await
    .unwrap();
}
