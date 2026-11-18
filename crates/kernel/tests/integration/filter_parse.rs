//! Lời thường thành bộ lọc (bước 52 mục 3): chỉ trả tham số hợp lệ theo schema bộ lọc của danh
//! sách, phần sai vào `ignored`; không có AI thì báo rõ; không chạy lệnh nào.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use autoxide_kernel::ai::TextModel;
use autoxide_testkit::ai::FakeTextModel;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use serde_json::json;

const LIST: &str = "kernel.list_logged_requests";

#[tokio::test]
async fn ai_filter_parse_chi_tra_tham_so_hop_le() {
    TestDatabase::run(async |test| {
        let model = Arc::new(FakeTextModel::new(|prompt| {
            assert!(prompt.instructions.contains("actor_kind"), "schema bộ lọc nằm trong luật");
            Ok(json!({
                "params": {"actor_kind": "agent", "status": "abc", "colour": "red", "limit": "5"},
                "ignored": ["tuần này"]
            }))
        }));
        let text: Arc<dyn TextModel> = model.clone();
        let app = TestApp::builder(test)
            .user("ann", 1, &[])
            .provide::<dyn TextModel>(text)
            .build()
            .await
            .unwrap();
        let parse = async |body| {
            app.post("/api/v1/kernel/filters/parse").token("ann").json(body).send().await.unwrap()
        };
        let parsed = parse(json!({"resource": LIST, "text": "lệnh agent gọi tuần này"})).await;
        assert_eq!(parsed.status(), StatusCode::OK, "{}", parsed.json());
        let data = parsed.json()["data"].clone();
        assert_eq!(data["params"], json!({"actor_kind": "agent"}), "chỉ tham số hợp lệ");
        assert_eq!(data["ignored"], json!(["tuần này", "colour=red", "status=abc"]));

        let unknown = parse(json!({"resource": "nope.list", "text": "x"})).await;
        assert_eq!(unknown.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let empty = parse(json!({"resource": LIST, "text": "  "})).await;
        assert_eq!(empty.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(model.calls(), 1, "đầu vào sai thì không gọi mô hình");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khong_co_ai_thi_bao_ro() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test).user("ann", 1, &[]).build().await.unwrap();
        let answer = app
            .post("/api/v1/kernel/filters/parse")
            .token("ann")
            .json(json!({"resource": LIST, "text": "đang chờ"}))
            .send()
            .await
            .unwrap();
        assert_eq!(
            (answer.status(), answer.json()["code"].as_str()),
            (StatusCode::SERVICE_UNAVAILABLE, Some("kernel.filters_need_ai"))
        );
    })
    .await
    .unwrap();
}
