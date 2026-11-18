//! Ghim của người dùng (bước 52 mục 4): ghim màn hình hay bản ghi, ghim lại không ra hai dòng,
//! tối đa 20, xếp lại, chỉ thấy và bỏ được ghim của chính mình.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use serde_json::{Value, json};

#[tokio::test]
async fn ghim_man_hinh_va_ban_ghi_toi_da_20() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test)
            .user("ann", 1, &[])
            .user("bob", 2, &[])
            .build()
            .await
            .unwrap();
        let pin = async |who: &str, body: Value| {
            app.post("/api/v1/me/pins").token(who).json(body).send().await.unwrap()
        };
        let list = async |who: &str| {
            app.get("/api/v1/me/pins").token(who).send().await.unwrap().json()["data"].clone()
        };

        let screen = pin("ann", json!({"kind": "navigation", "target": "home.work"})).await;
        assert_eq!(screen.status(), StatusCode::CREATED, "{}", screen.json());
        assert_eq!(screen.json()["data"]["label"], "Work and activity");
        let again = pin("ann", json!({"kind": "navigation", "target": "home.work"})).await;
        assert_eq!(again.status(), StatusCode::OK, "ghim lại không tạo dòng mới");
        assert_eq!(again.json()["data"]["id"], screen.json()["data"]["id"]);

        let unknown = pin("ann", json!({"kind": "navigation", "target": "nowhere.list"})).await;
        assert_eq!(unknown.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let unlabeled = pin(
            "ann",
            json!({"kind": "record", "target": "content.entry:0190a4c2-0000-7000-8000-000000000001"}),
        )
        .await;
        assert_eq!(unlabeled.json()["errors"][0]["field"], "label");
        let record = pin(
            "ann",
            json!({
                "kind": "record",
                "target": "content.entry:0190a4c2-0000-7000-8000-000000000001",
                "label": "Hello",
            }),
        )
        .await;
        assert_eq!(record.status(), StatusCode::CREATED);

        // Xếp lại: bản ghi lên trước.
        let ids: Vec<Value> = list("ann").await.as_array().unwrap().iter().map(|p| p["id"].clone()).collect();
        let reordered = app
            .put("/api/v1/me/pins/order")
            .token("ann")
            .json(json!({"ids": [ids[1], ids[0]]}))
            .send()
            .await
            .unwrap();
        assert_eq!(reordered.status(), StatusCode::OK, "{}", reordered.json());
        assert_eq!(reordered.json()["data"][0]["label"], "Hello");
        let partial = app
            .put("/api/v1/me/pins/order")
            .token("ann")
            .json(json!({"ids": [ids[0]]}))
            .send()
            .await
            .unwrap();
        assert_eq!(partial.status(), StatusCode::UNPROCESSABLE_ENTITY);

        // Người khác không thấy, không bỏ được.
        assert_eq!(list("bob").await, json!([]));
        let id = ids[0].as_str().unwrap();
        let foreign = app.delete(&format!("/api/v1/me/pins/{id}")).token("bob").send().await.unwrap();
        assert_eq!(foreign.status(), StatusCode::NOT_FOUND);

        // Tối đa 20.
        for n in 2..20 {
            let target = format!("content.entry:0190a4c2-0000-7000-8000-0000000000{n:02}");
            let made = pin("ann", json!({"kind": "record", "target": target, "label": "x"})).await;
            assert_eq!(made.status(), StatusCode::CREATED, "{n}");
        }
        let full = pin(
            "ann",
            json!({"kind": "record", "target": "content.entry:0190a4c2-0000-7000-8000-000000000099", "label": "x"}),
        )
        .await;
        assert_eq!(
            (full.status(), full.json()["code"].as_str()),
            (StatusCode::CONFLICT, Some("kernel.pin_limit_reached"))
        );

        let removed = app.delete(&format!("/api/v1/me/pins/{id}")).token("ann").send().await.unwrap();
        assert_eq!(removed.status(), StatusCode::NO_CONTENT);
        assert_eq!(list("ann").await.as_array().unwrap().len(), 19);
    })
    .await
    .unwrap();
}
