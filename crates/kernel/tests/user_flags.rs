//! Cờ theo người dùng qua HTTP (D-207): `GET` / `PUT` / `DELETE /api/v1/me/flags`, chỉ cờ của
//! chính người gọi, cần đăng nhập. Logic và test đồng thời ở `src/user_flags.rs`. Gate chạy file
//! này trên cả hai database.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
// Clippy chỉ tự miễn cho hàm `#[test]`, không miễn cho hàm phụ, nên khai ở đây.
// Trong `src/` thì KHÔNG được khai dòng này (`cargo xtask check` chặn).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use serde_json::json;

#[tokio::test]
async fn co_cua_toi_qua_http() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test).user("ada", 1, &[]).user("bob", 2, &[]).build().await.unwrap();
        let put = |who: &'static str, key: &str, value: &str| {
            app.request(Method::PUT, &format!("/api/v1/me/flags/{key}"))
                .token(who)
                .json(json!({"value": value}))
                .send()
        };

        let set = put("ada", "hint.saved_views", "dismissed").await.unwrap();
        assert_eq!(set.status(), StatusCode::OK, "{}", set.json());
        assert_eq!(set.json()["data"]["value"], "dismissed");
        put("ada", "hint.saved_views", "seen").await.unwrap();
        put("ada", "whats_new", "v0.13").await.unwrap();
        put("bob", "hint.pins", "dismissed").await.unwrap();

        let mine = app.request(Method::GET, "/api/v1/me/flags?prefix=hint.").token("ada").send().await.unwrap();
        assert_eq!(mine.status(), StatusCode::OK);
        assert_eq!(mine.json()["data"], json!([{"key": "hint.saved_views", "value": "seen", "updated_at": mine.json()["data"][0]["updated_at"]}]));

        let removed = app.request(Method::DELETE, "/api/v1/me/flags/whats_new").token("ada").send().await.unwrap();
        assert_eq!(removed.status(), StatusCode::NO_CONTENT);
        let again = app.request(Method::DELETE, "/api/v1/me/flags/whats_new").token("ada").send().await.unwrap();
        assert_eq!(again.status(), StatusCode::NO_CONTENT, "deleting twice is safe");

        let bad = put("ada", "Bad%20Key", "x").await.unwrap();
        assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let unknown = app
            .request(Method::PUT, "/api/v1/me/flags/ok")
            .token("ada")
            .json(json!({"value": "x", "other": 1}))
            .send()
            .await
            .unwrap();
        assert_eq!(unknown.status(), StatusCode::UNPROCESSABLE_ENTITY, "unknown fields are refused");

        let anonymous = app.request(Method::GET, "/api/v1/me/flags").send().await.unwrap();
        assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);

        let document = serde_json::to_value(app.openapi()).unwrap();
        let operation = &document["paths"]["/api/v1/me/flags/{key}"]["put"];
        assert_eq!(operation["tags"], json!(["me"]));
    })
    .await
    .unwrap();
}
