//! Sổ đối tượng trên chương trình thật (D-383), cả SQLite và Postgres: danh sách loại đối tượng kèm
//! cờ "có cây", lọc theo quyền đọc của module.

use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use serde_json::Value;

async fn app(test: &TestDatabase) -> TestApp {
    let mut builder = TestApp::builder(test)
        .user("editor", 1, &["content.read", "media.read"])
        .user("nobody", 2, &[]);
    for module in crate::modules::all() {
        builder = builder.boxed_module(module);
    }
    builder.build().await.unwrap()
}

#[tokio::test]
async fn loai_doi_tuong_kem_co_cay_theo_quyen_doc() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let answer = app.get("/api/v1/kernel/records/types").token("editor").send().await.unwrap();
        assert_eq!(answer.status(), StatusCode::OK);
        let types: Vec<Value> = answer.json()["data"].as_array().unwrap().clone();
        let find = |kind: &str| types.iter().find(|row| row["kind"] == kind).cloned();
        assert_eq!(find("content.term").unwrap()["has_children"], true);
        assert_eq!(find("content.entry").unwrap()["module"], "content");
        assert!(find("media.item").is_some());
        // Không có `organizations.read`: không thấy loại của module đó.
        assert!(find("organizations.unit").is_none());
        let kinds: Vec<&str> = types.iter().map(|row| row["kind"].as_str().unwrap()).collect();
        let mut sorted = kinds.clone();
        sorted.sort_unstable();
        assert_eq!(kinds, sorted);

        let answer = app.get("/api/v1/kernel/records/types").token("nobody").send().await.unwrap();
        assert_eq!(answer.status(), StatusCode::OK);
        assert!(answer.json()["data"].as_array().unwrap().is_empty());
    })
    .await
    .unwrap();
}
