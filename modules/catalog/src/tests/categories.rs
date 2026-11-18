//! Danh mục dạng cây: tạo, đổi tên, dời (không tạo vòng, cả khi hai lần dời chéo nhau chạy cùng
//! lúc), xoá khi rỗng; danh mục của sản phẩm kèm tổ tiên trong ảnh chụp biến thể.

use std::sync::Arc;

use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use serde_json::json;
use uuid::Uuid;

use super::{app, call, create_product};
use crate::api::Catalog;

async fn category(app: &TestApp, name: &str, parent: Option<&str>) -> String {
    let body = json!({"parent_id": parent, "texts": {"vi": {"name": name}}});
    let (status, made) = call(app, "POST", "/categories", "boss", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    made["data"]["id"].as_str().unwrap().to_owned()
}

async fn move_to(app: &TestApp, id: &str, parent: Option<&str>) -> (StatusCode, serde_json::Value) {
    let body = json!({"parent_id": parent});
    call(app, "POST", &format!("/categories/{id}/move"), "boss", Some(body)).await
}

#[tokio::test]
async fn doi_danh_muc_khong_tao_vong() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let fashion = category(&app, "Thời trang", None).await;
        let men = category(&app, "Nam", Some(&fashion)).await;
        let shirts = category(&app, "Áo sơ mi", Some(&men)).await;
        let (status, problem) = move_to(&app, &fashion, Some(&shirts)).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::CONFLICT, Some("catalog.category_cycle"))
        );
        let (status, problem) = move_to(&app, &men, Some(&men)).await;
        assert_eq!(status, StatusCode::CONFLICT, "{problem}");

        // Sản phẩm trong "Áo sơ mi": ảnh chụp có cả tổ tiên; lọc theo "Thời trang" thấy nó.
        let (product, variant) = create_product(&app, "boss", tree.a, "SHIRT", "Sơ mi").await;
        let body = json!({"category_ids": [shirts]});
        let (status, _) =
            call(&app, "PUT", &format!("/products/{product}/categories"), "boss", Some(body)).await;
        assert_eq!(status, StatusCode::OK);
        let catalog = app.api::<dyn Catalog>().unwrap();
        let id: Uuid = variant.parse().unwrap();
        let snapshot = catalog.variants(&app.default_tenant(), &[id], None).await.unwrap();
        let mut ids: Vec<String> = snapshot[0].category_ids.iter().map(Uuid::to_string).collect();
        ids.sort();
        let mut expected = vec![fashion.clone(), men.clone(), shirts.clone()];
        expected.sort();
        assert_eq!(ids, expected);
        let (_, listed) =
            call(&app, "GET", &format!("/products?category_id={fashion}"), "boss", None).await;
        assert_eq!(listed["data"].as_array().unwrap().len(), 1);

        // Dời "Nam" lên gốc: "Thời trang" không còn là tổ tiên.
        let (status, moved) = move_to(&app, &men, None).await;
        assert_eq!((status, moved["data"]["depth"].as_i64()), (StatusCode::OK, Some(0)));
        let snapshot = catalog.variants(&app.default_tenant(), &[id], None).await.unwrap();
        assert_eq!(snapshot[0].category_ids.len(), 2);
        let (_, listed) =
            call(&app, "GET", &format!("/products?category_id={fashion}"), "boss", None).await;
        assert_eq!(listed["data"].as_array().unwrap().len(), 0);

        // Danh mục còn sản phẩm hay danh mục con: không xoá được.
        let (status, problem) =
            call(&app, "DELETE", &format!("/categories/{men}"), "boss", None).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::CONFLICT, Some("catalog.category_not_empty"))
        );
        let (status, _) =
            call(&app, "DELETE", &format!("/categories/{fashion}"), "boss", None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        // Slug trùng trong cùng ngôn ngữ.
        let body = json!({"texts": {"vi": {"name": "Nam"}}});
        let (status, problem) = call(&app, "POST", "/categories", "boss", Some(body)).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::CONFLICT, Some("catalog.slug_taken"))
        );
    })
    .await
    .unwrap();
}

/// Hai lần dời chéo nhau cùng lúc (A vào dưới B, B vào dưới A): một bên `catalog.category_cycle`
/// hay cả hai hợp lệ — cây không bao giờ có vòng.
#[tokio::test]
async fn doi_danh_muc_dong_thoi() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        let app = Arc::new(app);
        for round in 0..4 {
            let a = category(&app, &format!("A{round}"), None).await;
            let b = category(&app, &format!("B{round}"), None).await;
            let (one, two) = {
                let (app1, app2) = (app.clone(), app.clone());
                let (a1, b1, a2, b2) = (a.clone(), b.clone(), a.clone(), b.clone());
                tokio::join!(
                    tokio::spawn(async move { move_to(&app1, &a1, Some(&b1)).await }),
                    tokio::spawn(async move { move_to(&app2, &b2, Some(&a2)).await }),
                )
            };
            let (one, two) = (one.unwrap(), two.unwrap());
            let ok = [&one, &two].iter().filter(|(status, _)| *status == StatusCode::OK).count();
            assert_eq!(ok, 1, "{one:?} {two:?}");
            for (status, body) in [one, two] {
                if status != StatusCode::OK {
                    assert_eq!(body["code"], "catalog.category_cycle", "{body}");
                }
            }
            // Không vòng: đi theo cha từ mỗi nút luôn tới gốc.
            let (_, listed) = call(&app, "GET", "/categories", "boss", None).await;
            let rows = listed["data"].as_array().unwrap();
            for row in rows {
                let mut current = row["parent_id"].as_str().map(str::to_owned);
                let mut steps = 0;
                while let Some(id) = current {
                    steps += 1;
                    assert!(steps < 10, "cycle at {row}");
                    current = rows
                        .iter()
                        .find(|other| other["id"] == id.as_str())
                        .and_then(|other| other["parent_id"].as_str().map(str::to_owned));
                }
            }
        }
    })
    .await
    .unwrap();
}
