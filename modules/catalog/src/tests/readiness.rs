//! Thuộc tính bắt buộc là dữ liệu có ngày hiệu lực, hàng cấm không mở bán được, hàng có điều kiện
//! cần giấy tờ còn hạn, `readiness` và số liệu đầu trang (D-223…D-227).

use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use chrono::{Duration, Utc};
use serde_json::{Value, json};

use super::{app, call, create_product};

/// Chạy `autoxide task catalog.load_restrictions` (gói `vn` phát hành kèm).
async fn load_pack(app: &TestApp) {
    let tasks = app.modules().tasks().unwrap();
    let (_, task) = tasks.iter().find(|(name, _)| name == "catalog.load_restrictions").unwrap();
    let out = task.run(&task.parse(&[]).unwrap()).await.unwrap();
    assert!(out.contains("restricted-goods rules"), "{out}");
}

async fn category(app: &TestApp, name: &str) -> String {
    let body = json!({"texts": {"vi": {"name": name}}});
    let (status, made) = call(app, "POST", "/categories", "boss", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    made["data"]["id"].as_str().unwrap().to_owned()
}

async fn in_category(app: &TestApp, product: &str, category: &str) {
    let body = json!({"category_ids": [category]});
    let (status, body) =
        call(app, "PUT", &format!("/products/{product}/categories"), "boss", Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

async fn publish(app: &TestApp, product: &str) -> (StatusCode, Value) {
    let body = json!({"status": "active"});
    call(app, "POST", &format!("/products/{product}/status"), "boss", Some(body)).await
}

async fn readiness(app: &TestApp, product: &str) -> String {
    let (_, got) = call(app, "GET", &format!("/products/{product}"), "boss", None).await;
    got["data"]["readiness"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn chuyen_trang_thai_kiem_thuoc_tinh_bat_buoc() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        load_pack(&app).await;
        // Gói nạp thuộc tính nhãn hàng hoá mẫu cho khách.
        let (_, attributes) = call(&app, "GET", "/attributes", "boss", None).await;
        let origin = attributes["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["code"] == "origin")
            .unwrap()["id"]
            .clone();
        let food = category(&app, "Thực phẩm").await;
        let today = Utc::now().date_naive();
        let body = json!({"attributes": [{"attribute_id": origin, "required": true, "effective_from": today}]});
        let (status, saved) =
            call(&app, "PUT", &format!("/categories/{food}/attributes"), "boss", Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{saved}");

        let (product, _) = create_product(&app, "boss", tree.a, "FOOD-1", "Bánh quy").await;
        in_category(&app, &product, &food).await;
        assert_eq!(readiness(&app, &product).await, "missing_attributes");
        let (status, problem) = publish(&app, &product).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        assert_eq!(problem["code"], "catalog.required_attributes_missing");
        assert!(problem["detail"].as_str().unwrap().contains("origin"), "{problem}");

        // Số liệu đầu trang theo `readiness`.
        let (_, counts) = call(&app, "GET", "/products/counts?group_by=readiness", "boss", None).await;
        let missing = counts["data"].as_array().unwrap().iter().find(|c| c["key"] == "missing_attributes").unwrap();
        assert_eq!(missing["count"], 1);

        let body = json!({"attributes": [{"attribute_id": origin, "value": "Việt Nam"}]});
        let (status, values) =
            call(&app, "PUT", &format!("/products/{product}/attributes"), "boss", Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{values}");
        assert_eq!(readiness(&app, &product).await, "ready");
        let (status, sold) = publish(&app, &product).await;
        assert_eq!(status, StatusCode::OK, "{sold}");
        let (_, filtered) = call(&app, "GET", "/products?readiness=ready&status=active", "boss", None).await;
        assert_eq!(filtered["data"].as_array().unwrap().len(), 1);
        // Sai kiểu giá trị: báo theo ô.
        let body = json!({"attributes": [{"attribute_id": origin, "value": 42}]});
        let (status, _) =
            call(&app, "PUT", &format!("/products/{product}/attributes"), "boss", Some(body)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}

/// Thuộc tính bắt buộc theo ngày hiệu lực: quy tắc chưa tới ngày hay đã hết hạn không chặn.
#[tokio::test]
async fn thuoc_tinh_bat_buoc_theo_ngay_hieu_luc() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let body = json!({"code": "material", "data_type": "option", "names": {"vi": "Chất liệu"}});
        let (status, made) = call(&app, "POST", "/attributes", "boss", Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        let material = made["data"]["id"].as_str().unwrap().to_owned();
        let options = json!({"options": [{"code": "cotton", "labels": {"vi": "Cotton"}}]});
        let (status, _) =
            call(&app, "PUT", &format!("/attributes/{material}/options"), "boss", Some(options)).await;
        assert_eq!(status, StatusCode::OK);

        let parent = category(&app, "Thời trang").await;
        let child = {
            let body = json!({"parent_id": parent, "texts": {"vi": {"name": "Áo"}}});
            let (_, made) = call(&app, "POST", "/categories", "boss", Some(body)).await;
            made["data"]["id"].as_str().unwrap().to_owned()
        };
        let today = Utc::now().date_naive();
        let rule = |from: chrono::NaiveDate, to: Option<chrono::NaiveDate>| {
            json!({"attributes": [{"attribute_id": material, "required": true, "applies_to_descendants": true, "effective_from": from, "effective_to": to}]})
        };
        let (product, _) = create_product(&app, "boss", tree.a, "SHIRT-X", "Áo").await;
        in_category(&app, &product, &child).await;
        let uri = format!("/categories/{parent}/attributes");
        // Hiệu lực từ tuần sau: chưa chặn.
        let (status, _) = call(&app, "PUT", &uri, "boss", Some(rule(today + Duration::days(7), None))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(readiness(&app, &product).await, "ready");
        // Đã hết hạn hôm qua: không chặn.
        let (status, _) = call(
            &app,
            "PUT",
            &uri,
            "boss",
            Some(rule(today - Duration::days(30), Some(today - Duration::days(1)))),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(readiness(&app, &product).await, "ready");
        // Đang hiệu lực, áp cho danh mục con: chặn tới khi có giá trị.
        let (status, _) = call(&app, "PUT", &uri, "boss", Some(rule(today, None))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(readiness(&app, &product).await, "missing_attributes");
        let body = json!({"attributes": [{"attribute_id": material, "value": "cotton"}]});
        let (status, _) =
            call(&app, "PUT", &format!("/products/{product}/attributes"), "boss", Some(body)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(readiness(&app, &product).await, "ready");
        // Bỏ giá trị chọn đang được dùng: chặn.
        let (status, problem) = call(
            &app,
            "PUT",
            &format!("/attributes/{material}/options"),
            "boss",
            Some(json!({"options": []})),
        )
        .await;
        assert_eq!((status, problem["code"].as_str()), (StatusCode::CONFLICT, Some("catalog.attribute_option_in_use")));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hang_cam_khong_mo_ban_duoc() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        load_pack(&app).await;
        let (_, rules) = call(&app, "GET", "/restriction-rules", "boss", None).await;
        assert!(
            rules["data"].as_array().unwrap().iter().any(|rule| rule["code"] == "firecrackers")
        );
        let banned = category(&app, "Pháo").await;
        let body = json!({"rule_codes": ["firecrackers"]});
        let (status, _) =
            call(&app, "PUT", &format!("/categories/{banned}/restrictions"), "boss", Some(body))
                .await;
        assert_eq!(status, StatusCode::OK);
        let (status, problem) = call(
            &app,
            "PUT",
            &format!("/categories/{banned}/restrictions"),
            "boss",
            Some(json!({"rule_codes": ["no_such_rule"]})),
        )
        .await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("catalog.restriction_rule_not_found"))
        );
        let (product, _) = create_product(&app, "boss", tree.a, "BANG-1", "Pháo hoa").await;
        in_category(&app, &product, &banned).await;
        assert_eq!(readiness(&app, &product).await, "restricted");
        let (status, problem) = publish(&app, &product).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::CONFLICT, Some("catalog.prohibited_goods"))
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hang_co_dieu_kien_can_giay_to_con_han() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        load_pack(&app).await;
        let wine = category(&app, "Rượu").await;
        let body = json!({"rule_codes": ["alcohol"]});
        let (status, _) =
            call(&app, "PUT", &format!("/categories/{wine}/restrictions"), "boss", Some(body)).await;
        assert_eq!(status, StatusCode::OK);
        let (product, _) = create_product(&app, "boss", tree.a, "WINE-1", "Rượu vang").await;
        in_category(&app, &product, &wine).await;
        assert_eq!(readiness(&app, &product).await, "missing_documents");
        let (status, problem) = publish(&app, &product).await;
        assert_eq!((status, problem["code"].as_str()), (StatusCode::CONFLICT, Some("catalog.documents_required")));

        let today = Utc::now().date_naive();
        let documents = format!("/products/{product}/documents");
        // Giấy phép đã hết hạn: vẫn thiếu.
        let expired = json!({
            "document_type": "alcohol_retail_licence", "number": "01/GP", "issued_by": "Sở Công Thương",
            "issued_on": today - Duration::days(400), "expires_on": today - Duration::days(1),
        });
        let (status, _) = call(&app, "POST", &documents, "boss", Some(expired)).await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(readiness(&app, &product).await, "missing_documents");
        let valid = json!({
            "document_type": "alcohol_retail_licence", "number": "02/GP", "issued_by": "Sở Công Thương",
            "issued_on": today - Duration::days(10), "expires_on": today + Duration::days(365),
        });
        let (status, made) = call(&app, "POST", &documents, "boss", Some(valid)).await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        assert_eq!(readiness(&app, &product).await, "ready");
        let (status, sold) = publish(&app, &product).await;
        assert_eq!(status, StatusCode::OK, "{sold}");
        // Nhân viên cửa hàng B không thấy giấy tờ của cửa hàng A.
        let (status, _) = call(&app, "GET", &documents, "store_b", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, listed) = call(&app, "GET", &documents, "boss", None).await;
        assert_eq!((status, listed["data"].as_array().unwrap().len()), (StatusCode::OK, 2));
        let id = made["data"]["id"].as_str().unwrap();
        let (status, _) = call(&app, "DELETE", &format!("{documents}/{id}"), "boss", None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert_eq!(readiness(&app, &product).await, "missing_documents");
    })
    .await
    .unwrap();
}
