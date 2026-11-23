//! `GET /api/v1/kernel/find` từ chối câu tìm quá dài bằng lỗi kiểm dữ liệu (rà soát bảo mật 2026-11):
//! câu dài vô hạn thành mẫu `LIKE` chạy trên mọi nguồn.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_testkit::{TestApp, TestDatabase};

#[tokio::test]
async fn cau_tim_qua_dai_bi_tu_choi() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test).user("ada", 1, &[]).build().await.unwrap();
        let long = "a".repeat(201);
        let answer = app
            .get(&format!("/api/v1/kernel/find?q={long}"))
            .token("ada")
            .unchecked()
            .send()
            .await
            .unwrap();
        assert_eq!(answer.status().as_u16(), 422);
        let body = answer.json();
        assert_eq!(body["errors"][0]["field"], "q", "{body}");
        assert_eq!(body["errors"][0]["code"], "length", "{body}");
        let ok = app.get("/api/v1/kernel/find?q=ab").token("ada").send().await.unwrap();
        assert_eq!(ok.status().as_u16(), 200);
    })
    .await
    .unwrap();
}
