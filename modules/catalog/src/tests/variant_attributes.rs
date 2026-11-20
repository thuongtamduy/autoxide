//! Thông tin theo biến thể trên trang sản phẩm (website bán hàng): thuộc tính là dữ liệu của khách,
//! chữ theo ngôn ngữ, mỗi biến thể một bộ.

use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use serde_json::{Value, json};

use super::public::{drain, on_sale, price_list, public};
use super::{app, call};

async fn attribute(app: &autoxide_testkit::TestApp, code: &str, data_type: &str) -> Value {
    let body = json!({"code": code, "data_type": data_type, "names": {"vi": code, "en": code}});
    let (status, made) = call(app, "POST", "/attributes", "boss", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    made["data"]["id"].clone()
}

#[tokio::test]
async fn thong_tin_theo_bien_the_tren_trang_san_pham() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let serves = attribute(&app, "serves", "integer").await;
        let care = attribute(&app, "care", "text").await;
        let dishwasher = attribute(&app, "dishwasher_safe", "boolean").await;
        let list = price_list(&app, tree.chain).await;
        let (product, variant) = on_sale(&app, &list, tree.chain, "BOWL-1", "Bát sứ", 1).await;
        let body = json!({"name": "Porcelain bowl"});
        let (status, _) =
            call(&app, "PUT", &format!("/products/{product}/translations/en"), "boss", Some(body))
                .await;
        assert_eq!(status, StatusCode::OK);

        let uri = format!("/variants/{variant}/attributes");
        let body = json!({"attributes": [
            {"attribute_id": serves, "value": 4},
            {"attribute_id": care, "value": "Rửa tay", "texts": {"en": "Hand wash"}},
            {"attribute_id": dishwasher, "value": false},
        ]});
        let (status, saved) = call(&app, "PUT", &uri, "boss", Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        assert_eq!(saved["data"].as_array().unwrap().len(), 3);
        let (_, read) = call(&app, "GET", &uri, "boss", None).await;
        assert_eq!(read["data"], saved["data"]);

        // Sai: bản dịch cho thuộc tính không phải chữ, ngôn ngữ chưa cấu hình; gom lỗi một lần.
        let bad = json!({"attributes": [
            {"attribute_id": serves, "value": 2, "texts": {"en": "two"}},
            {"attribute_id": care, "value": "x", "texts": {"fr": "x"}},
        ]});
        let (status, problem) = call(&app, "PUT", &uri, "boss", Some(bad)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        assert_eq!(problem["errors"].as_array().unwrap().len(), 2, "{problem}");
        // Cửa hàng B không sửa được.
        let (status, _) = call(&app, "PUT", &uri, "store_b", Some(json!({"attributes": []}))).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        drain(&app).await;
        let site = tree.chain.as_uuid();
        let page = |slug: &'static str, locale: &'static str| {
            let app = &app;
            async move {
                let uri = format!("/products/by-slug/{slug}?locale={locale}&site={site}");
                let (status, page, _) = public(app, &uri, None).await;
                assert_eq!(status, StatusCode::OK, "{page}");
                page["data"]["variants"][0]["attributes"].clone()
            }
        };
        let labels = |attributes: &Value| -> Vec<(String, String)> {
            attributes
                .as_array()
                .unwrap()
                .iter()
                .map(|item| {
                    (
                        item["code"].as_str().unwrap().to_owned(),
                        item["label"].as_str().unwrap().to_owned(),
                    )
                })
                .collect()
        };
        let vi = page("bat-su", "vi").await;
        assert_eq!(
            labels(&vi),
            vec![
                ("serves".to_owned(), "4".to_owned()),
                ("care".to_owned(), "Rửa tay".to_owned()),
                ("dishwasher_safe".to_owned(), "false".to_owned()),
            ]
        );
        let en = page("porcelain-bowl", "en").await;
        assert_eq!(labels(&en)[1], ("care".to_owned(), "Hand wash".to_owned()));
    })
    .await
    .unwrap();
}
