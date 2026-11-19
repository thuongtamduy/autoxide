//! Tìm cho website (D-201): từ đồng nghĩa, gợi ý hai tầng dấu, số đếm theo giá trị lọc.

use axum::http::StatusCode;
use serde_json::{Value, json};

use super::public::{drain, names, on_sale, price_list, public};
use super::{app, call, create_product};

/// Từ đồng nghĩa của khách: tìm "áo phông" ra "Áo thun"; lưu dạng bỏ dấu; nhóm sai thì 422.
#[tokio::test]
async fn tu_dong_nghia_tim_ra_tu_khac() {
    autoxide_testkit::TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let list = price_list(&app, tree.chain).await;
        on_sale(&app, &list, tree.chain, "S-1", "Áo thun trắng", 100_000).await;
        drain(&app).await;
        let site = tree.chain.as_uuid();
        let uri = format!("/products?locale=vi&site={site}&q=%C3%A1o%20ph%C3%B4ng");
        let (_, before, _) = public(&app, &uri, None).await;
        assert!(names(&before).is_empty());

        let body = json!({"groups": [["Áo phông", "áo thun", "tee"]]});
        let (status, saved) = call(&app, "PUT", "/synonyms", "boss", Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        assert_eq!(saved["data"]["groups"], json!([["ao phong", "ao thun", "tee"]]));
        let (_, read) = call(&app, "GET", "/synonyms", "boss", None).await;
        assert_eq!(read["data"], saved["data"]);
        let (_, after, _) = public(&app, &uri, None).await;
        assert_eq!(names(&after), vec!["Áo thun trắng"]);

        let bad = json!({"groups": [["chỉ một"], ["", "x"]]});
        let (status, problem) = call(&app, "PUT", "/synonyms", "boss", Some(bad)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        assert_eq!(problem["errors"].as_array().unwrap().len(), 3, "{problem}");
    })
    .await
    .unwrap();
}

/// Gợi ý: khớp đúng dấu xếp trước khớp không dấu; chỉ sản phẩm đang bán; có danh mục.
#[tokio::test]
async fn goi_y_khop_dung_dau_truoc() {
    autoxide_testkit::TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let list = price_list(&app, tree.chain).await;
        on_sale(&app, &list, tree.chain, "D-1", "Dâu tây sấy", 50_000).await;
        on_sale(&app, &list, tree.chain, "D-2", "Dầu ăn", 40_000).await;
        create_product(&app, "boss", tree.chain, "D-3", "Dầu gội nháp").await;
        let body = json!({"texts": {"vi": {"name": "Dầu và gia vị"}}});
        let (status, _) = call(&app, "POST", "/categories", "boss", Some(body)).await;
        assert_eq!(status, StatusCode::CREATED);
        drain(&app).await;

        let site = tree.chain.as_uuid();
        let uri = format!("/search/suggest?locale=vi&site={site}&q=d%E1%BA%A7u");
        let (status, found, _) = public(&app, &uri, None).await;
        assert_eq!(status, StatusCode::OK, "{found}");
        let items: Vec<(String, String, bool)> = found["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| {
                (
                    item["kind"].as_str().unwrap().to_owned(),
                    item["label"].as_str().unwrap().to_owned(),
                    item["exact"].as_bool().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            items,
            vec![
                ("catalog.product".to_owned(), "Dầu ăn".to_owned(), true),
                ("catalog.category".to_owned(), "Dầu và gia vị".to_owned(), true),
                ("catalog.product".to_owned(), "Dâu tây sấy".to_owned(), false),
            ],
            "{found}"
        );
        assert_eq!(found["data"][0]["path"], "/products/dau-an");
        let (_, few, _) = public(&app, &format!("{uri}&limit=1"), None).await;
        assert_eq!(few["data"].as_array().unwrap().len(), 1);
        let (status, _, _) = public(&app, "/search/suggest?locale=vi&q=", None).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let (status, _, _) = public(&app, &format!("{uri}&limit=11"), None).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}

fn counts(facet: &Value) -> Vec<(String, u64)> {
    facet["values"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| (item["value"].as_str().unwrap().to_owned(), item["count"].as_u64().unwrap()))
        .collect()
}

/// Số đếm theo giá trị: thuộc tính lựa chọn và người bán; đếm của một bộ lọc không lọc theo
/// chính nó.
#[tokio::test]
async fn facet_dem_theo_gia_tri() {
    autoxide_testkit::TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let list = price_list(&app, tree.chain).await;
        let body = json!({"code": "color", "data_type": "option", "names": {"vi": "Màu"}, "filterable": true});
        let (_, made) = call(&app, "POST", "/attributes", "boss", Some(body)).await;
        let color = made["data"]["id"].as_str().unwrap().to_owned();
        let options = json!({"options": [
            {"code": "red", "labels": {"vi": "Đỏ"}},
            {"code": "blue", "labels": {"vi": "Xanh"}},
        ]});
        let (status, _) =
            call(&app, "PUT", &format!("/attributes/{color}/options"), "boss", Some(options)).await;
        assert_eq!(status, StatusCode::OK);
        let goods = [(tree.chain, "F-1", "red"), (tree.chain, "F-2", "red"), (tree.a, "F-3", "blue")];
        for (org, sku, colour) in goods {
            let (product, _) = on_sale(&app, &list, org, sku, &format!("Hàng {sku}"), 10_000).await;
            let body = json!({"attributes": [{"attribute_id": color, "value": colour}]});
            let (status, _) =
                call(&app, "PUT", &format!("/products/{product}/attributes"), "boss", Some(body)).await;
            assert_eq!(status, StatusCode::OK);
        }
        drain(&app).await;

        let site = tree.chain.as_uuid();
        let (status, all, _) =
            public(&app, &format!("/products/facets?locale=vi&site={site}"), None).await;
        assert_eq!(status, StatusCode::OK, "{all}");
        let data = &all["data"];
        assert_eq!(data["total"], 3);
        assert_eq!(data["attributes"][0]["code"], "color");
        assert_eq!(data["attributes"][0]["name"], "Màu");
        assert_eq!(counts(&data["attributes"][0]), vec![("red".to_owned(), 2), ("blue".to_owned(), 1)]);
        assert_eq!(data["attributes"][0]["values"][0]["label"], "Đỏ");
        let mut sellers = counts(&json!({"values": data["sellers"]}));
        sellers.sort();
        let mut expected =
            vec![(tree.chain.as_uuid().to_string(), 2), (tree.a.as_uuid().to_string(), 1)];
        expected.sort();
        assert_eq!(sellers, expected);

        // Lọc màu đỏ: tổng 2, nhưng số đếm màu vẫn tính cả xanh; người bán chỉ còn chuỗi.
        let uri = format!("/products/facets?locale=vi&site={site}&attr.color=red");
        let (_, red, _) = public(&app, &uri, None).await;
        assert_eq!(red["data"]["total"], 2);
        assert_eq!(counts(&red["data"]["attributes"][0]).len(), 2);
        assert_eq!(red["data"]["sellers"].as_array().unwrap().len(), 1);
        // Danh sách và số đếm khớp nhau.
        let (_, listed, _) =
            public(&app, &format!("/products?locale=vi&site={site}&attr.color=red"), None).await;
        assert_eq!(names(&listed).len(), 2);
        let (status, _, _) =
            public(&app, &format!("/products/facets?locale=vi&site={site}&attr.size=m"), None).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}
