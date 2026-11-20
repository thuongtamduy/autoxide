//! Nhập / xuất CSV bằng job: lỗi theo dòng không chặn dòng đúng, nhập lại không nhân đôi, xuất
//! chặn CSV injection và nhập lại được đúng giá trị.

use std::sync::Arc;

use autoxide_kernel::storage::Storage;
use autoxide_testkit::{MemoryStorage, TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

use super::public::drain;
use super::{app, app_with, call};

async fn with_storage(test: &TestDatabase) -> TestApp {
    let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
    app_with(test, move |builder| builder.provide::<dyn Storage>(storage)).await.0
}

/// Gửi file nhập; chạy job; trả lần nhập đã xong.
async fn import(app: &TestApp, csv: &str) -> Value {
    let answer = app
        .request(Method::POST, "/api/v1/catalog/imports")
        .token("boss")
        .header("content-type", "text/csv")
        .bytes(csv.as_bytes().to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(answer.status(), StatusCode::CREATED, "{}", answer.text());
    let id = answer.json()["data"]["id"].as_str().unwrap().to_owned();
    drain(app).await;
    let (status, found) = call(app, "GET", &format!("/imports/{id}"), "boss", None).await;
    assert_eq!(status, StatusCode::OK, "{found}");
    found["data"].clone()
}

async fn names(app: &TestApp) -> Vec<(String, String)> {
    let (_, listed) = call(app, "GET", "/products?limit=50", "boss", None).await;
    let mut names: Vec<(String, String)> = listed["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (row["name"].as_str().unwrap().to_owned(), row["status"].as_str().unwrap().to_owned())
        })
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn nhap_bao_loi_theo_dong() {
    TestDatabase::run(async |test| {
        let app = with_storage(test).await;
        let csv = "sku,name,tax_category,weight_grams\r\n\
                   A-1,Áo thun,vat.standard,200\r\n\
                   A-2,Thiếu nhóm thuế,,\r\n\
                   A-3,Cân sai,vat.standard,nặng\r\n\
                   ,,,\r\n\
                   A-4,\"Quần, dài\",vat.standard,\r\n";
        let done = import(&app, csv).await;
        assert_eq!(done["status"], "succeeded_with_errors", "{done}");
        assert_eq!(done["total_rows"], 4);
        assert_eq!(done["ok_rows"], 2);
        let errors: Vec<(i64, String)> = done["errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| (e["row"].as_i64().unwrap(), e["field"].as_str().unwrap().to_owned()))
            .collect();
        assert_eq!(errors, vec![(3, "tax_category".to_owned()), (4, "weight_grams".to_owned())]);
        // Sản phẩm mới là nháp: nhập không bao giờ mở bán.
        assert_eq!(
            names(&app).await,
            vec![
                ("Quần, dài".to_owned(), "draft".to_owned()),
                ("Áo thun".to_owned(), "draft".to_owned())
            ]
        );

        // File sai định dạng: từ chối cả file, không ghi dòng nào.
        let rejected = import(&app, "sku,price\r\nB-1,10\r\n").await;
        assert_eq!(rejected["status"], "rejected");
        assert_eq!(rejected["errors"][0]["row"], 0);
        assert_eq!(names(&app).await.len(), 2);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nhap_lai_cung_file_khong_nhan_doi() {
    TestDatabase::run(async |test| {
        let app = with_storage(test).await;
        let csv = "sku,name,tax_category\r\nN-1,Nồi,vat.standard\r\n";
        assert_eq!(import(&app, csv).await["status"], "succeeded");
        let (_, before) = call(&app, "GET", "/products", "boss", None).await;
        let id = before["data"][0]["id"].as_str().unwrap().to_owned();
        let (_, detail) = call(&app, "GET", &format!("/products/{id}"), "boss", None).await;
        let slug = detail["data"]["translations"][0]["slug"].clone();
        assert_eq!(import(&app, csv).await["status"], "succeeded");
        // Cùng SKU, tên mới: sửa sản phẩm cũ, giữ đường dẫn.
        let again = "sku,name,barcode\r\nN-1,Nồi cơm,4006381333931\r\n";
        let done = import(&app, again).await;
        assert_eq!(done["status"], "succeeded", "{done}");
        assert_eq!(names(&app).await, vec![("Nồi cơm".to_owned(), "draft".to_owned())]);
        let (_, detail) = call(&app, "GET", &format!("/products/{id}"), "boss", None).await;
        assert_eq!(detail["data"]["translations"][0]["slug"], slug);
        assert_eq!(detail["data"]["variants"][0]["barcode"], "4006381333931");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn xuat_chan_csv_injection() {
    TestDatabase::run(async |test| {
        let app = with_storage(test).await;
        let csv = "sku,name,tax_category\r\nX-1,\"=HYPERLINK(\"\"http://x\"\")\",vat.standard\r\n";
        assert_eq!(import(&app, csv).await["status"], "succeeded");
        let (status, made) = call(&app, "POST", "/exports", "boss", Some(json!({}))).await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        let id = made["data"]["id"].as_str().unwrap().to_owned();
        drain(&app).await;
        let (_, done) = call(&app, "GET", &format!("/exports/{id}"), "boss", None).await;
        assert_eq!(done["data"]["status"], "succeeded", "{done}");
        assert_eq!(done["data"]["total_rows"], 1);
        let path = done["data"]["content_path"].as_str().unwrap().to_owned();
        let answer = app.request(Method::GET, &path).token("boss").send().await.unwrap();
        assert_eq!(answer.status(), StatusCode::OK);
        let text = answer.text();
        assert!(text.contains("\"'=HYPERLINK(\"\"http://x\"\")\""), "{text}");
        // Người khác không thấy bản xuất của mình.
        let (status, _) = call(&app, "GET", &format!("/exports/{id}"), "store_a", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        // Nhập lại file vừa xuất: tên giữ nguyên, không có `'`, không thêm sản phẩm.
        assert_eq!(import(&app, &text).await["status"], "succeeded");
        assert_eq!(
            names(&app).await,
            vec![("=HYPERLINK(\"http://x\")".to_owned(), "draft".to_owned())]
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nhap_khong_co_noi_luu_file() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        let answer = app
            .request(Method::POST, "/api/v1/catalog/imports")
            .token("boss")
            .header("content-type", "text/csv")
            .bytes(b"sku\r\nA\r\n".to_vec())
            .send()
            .await
            .unwrap();
        assert_eq!(answer.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(answer.json()["code"], "catalog.storage_unavailable");
    })
    .await
    .unwrap();
}
