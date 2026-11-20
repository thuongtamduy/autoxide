//! Bản nháp AI cho mô tả và SEO (D-109): chỉ là bản nháp; áp thì vào bản dịch và bật
//! `ai_assisted`; áp / bỏ một lần.

use std::sync::Arc;

use autoxide_kernel::ai::TextModel;
use autoxide_testkit::TestDatabase;
use autoxide_testkit::ai::FakeTextModel;
use axum::http::StatusCode;
use serde_json::json;

use super::{app, app_with, call, create_product};

fn model() -> Arc<FakeTextModel> {
    Arc::new(FakeTextModel::new(|_| {
        Ok(json!({
            "short_description": "Áo thun cotton thoáng mát.",
            "description": "Vải cotton.\n\nGiặt máy được.",
            "meta_title": "Áo thun cotton",
            "meta_description": "Áo thun cotton thoáng mát cho ngày hè, giặt máy được, nhiều cỡ."
        }))
    }))
}

#[tokio::test]
async fn goi_y_chi_la_ban_nhap_ap_thi_bat_ai_assisted() {
    TestDatabase::run(async |test| {
        let model = model();
        let provided = model.clone();
        let (app, tree) =
            app_with(test, move |builder| builder.provide::<dyn TextModel>(provided)).await;
        let (product, _) = create_product(&app, "boss", tree.a, "AI-1", "Áo thun").await;
        let uri = format!("/products/{product}/suggestions");
        let (status, made) = call(&app, "POST", &uri, "boss", Some(json!({"locale": "vi"}))).await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        assert_eq!(made["data"]["status"], "pending");
        assert_eq!(made["data"]["meta_title"], "Áo thun cotton");
        assert_eq!(model.calls(), 1);
        // Bản nháp chưa đổi gì trên sản phẩm.
        let (_, detail) = call(&app, "GET", &format!("/products/{product}"), "boss", None).await;
        assert_eq!(detail["data"]["ai_assisted"], false);
        assert_eq!(detail["data"]["translations"][0]["short_description"], "");
        let slug = detail["data"]["translations"][0]["slug"].clone();

        // Cửa hàng B không thấy bản nháp của sản phẩm cửa hàng A.
        let id = made["data"]["id"].as_str().unwrap().to_owned();
        let (status, _) =
            call(&app, "POST", &format!("/suggestions/{id}/apply"), "store_b", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, applied) =
            call(&app, "POST", &format!("/suggestions/{id}/apply"), "boss", None).await;
        assert_eq!(status, StatusCode::OK, "{applied}");
        assert_eq!(applied["data"]["status"], "applied");
        let (_, detail) = call(&app, "GET", &format!("/products/{product}"), "boss", None).await;
        assert_eq!(detail["data"]["ai_assisted"], true);
        let text = &detail["data"]["translations"][0];
        assert_eq!(text["name"], "Áo thun");
        assert_eq!(text["slug"], slug);
        assert_eq!(text["short_description"], "Áo thun cotton thoáng mát.");
        assert_eq!(text["meta_title"], "Áo thun cotton");
        // Áp lần hai, hay bỏ sau khi áp: 409.
        for action in ["apply", "discard"] {
            let (status, problem) =
                call(&app, "POST", &format!("/suggestions/{id}/{action}"), "boss", None).await;
            assert_eq!(status, StatusCode::CONFLICT, "{problem}");
            assert_eq!(problem["code"], "catalog.suggestion_decided");
        }
        // Bỏ một bản nháp khác: sản phẩm không đổi.
        let (_, other) = call(&app, "POST", &uri, "boss", Some(json!({"locale": "vi"}))).await;
        let other = other["data"]["id"].as_str().unwrap().to_owned();
        let (status, discarded) =
            call(&app, "POST", &format!("/suggestions/{other}/discard"), "boss", None).await;
        assert_eq!(status, StatusCode::OK, "{discarded}");
        assert_eq!(discarded["data"]["status"], "discarded");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khong_co_ai_thi_503() {
    TestDatabase::run(async |test| {
        let (app, tree) = app(test).await;
        let (product, _) = create_product(&app, "boss", tree.a, "AI-2", "Ấm").await;
        let uri = format!("/products/{product}/suggestions");
        let (status, problem) =
            call(&app, "POST", &uri, "boss", Some(json!({"locale": "vi"}))).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{problem}");
        assert_eq!(problem["code"], "catalog.ai_unavailable");
    })
    .await
    .unwrap();
}
