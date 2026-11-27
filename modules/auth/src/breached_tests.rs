//! Bước 71 G7 (D-20261127-breached-passwords): mật khẩu phổ biến / đã lộ bị chặn ở MỌI đường đặt mật khẩu qua
//! REST — tự đăng ký, quản trị tạo tài khoản, đặt mật khẩu đầu tiên, đổi mật khẩu, đặt lại bằng mail — với
//! cùng mã `common_password` cho trường, không phân biệt hoa thường; mật khẩu khó đoán thì qua. Lời mời có
//! bài riêng trong `invitation.rs`, dòng lệnh trong `crates/app/tests/integration/cli.rs`. Chạy trên SQLite và
//! Postgres.

use autoxide_kernel::event::Actor;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use serde_json::{Value, json};

use crate::http::test_support::{PASSWORD, app, app_with, bearer, call};
use crate::testing::Services;
use crate::user::NewUser;

/// Có trong danh sách (`password123456789`), viết hoa khác đi: vẫn bị chặn.
const BREACHED: &str = "PassWord123456789";
const GOOD: &str = "a long and unusual passphrase";

/// 422 với đúng một lỗi `common_password` ở trường `field`, câu báo không nhắc lại mật khẩu.
fn assert_refused(status: StatusCode, problem: &Value, field: &str) {
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
    assert_eq!(problem["code"], "validation_failed", "{problem}");
    assert_eq!(problem["errors"][0]["field"], field, "{problem}");
    assert_eq!(problem["errors"][0]["code"], "common_password", "{problem}");
    assert!(!problem.to_string().to_lowercase().contains("password123456789"), "{problem}");
}

/// Phiên của một quản trị viên mới.
async fn admin_session(app: &TestApp, services: &Services) -> String {
    let boss = NewUser {
        email: "boss@example.com".to_owned(),
        name: "Boss".to_owned(),
        password: Some(PASSWORD.to_owned()),
    };
    let tenant = &services.tenant;
    services.users.create_with_roles(tenant, Actor::system(), boss, true).await.unwrap();
    bearer(app, "boss@example.com", PASSWORD).await
}

#[tokio::test]
async fn tu_dang_ky_chan_mat_khau_da_lo() {
    TestDatabase::run(async |test| {
        let (app, _) = app_with(test, true).await;
        let body = json!({"email": "bob@example.com", "name": "Bob", "password": BREACHED});
        let (status, _, problem) = call(&app, "POST", "/register", None, Some(body)).await;
        assert_refused(status, &problem, "password");

        let body = json!({"email": "bob@example.com", "name": "Bob", "password": GOOD});
        let (status, _, created) = call(&app, "POST", "/register", None, Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn quan_tri_tao_tai_khoan_va_dat_mat_khau_dau_chan_mat_khau_da_lo() {
    TestDatabase::run(async |test| {
        let (app, services) = app(test).await;
        let admin = admin_session(&app, &services).await;
        let body = json!({"email": "cy@example.com", "name": "Cy", "password": BREACHED});
        let (status, _, problem) = call(&app, "POST", "/users", Some(&admin), Some(body)).await;
        assert_refused(status, &problem, "password");

        // Tạo không mật khẩu, rồi đặt mật khẩu đầu tiên.
        let body = json!({"email": "cy@example.com", "name": "Cy"});
        let (status, _, created) = call(&app, "POST", "/users", Some(&admin), Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let path = format!("/users/{}/password", created["data"]["id"].as_str().unwrap());
        let body = json!({"password": BREACHED});
        let (status, _, problem) = call(&app, "PUT", &path, Some(&admin), Some(body)).await;
        assert_refused(status, &problem, "password");
        let body = json!({"password": GOOD});
        let (status, _, set) = call(&app, "PUT", &path, Some(&admin), Some(body)).await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{set}");
        bearer(&app, "cy@example.com", GOOD).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn doi_mat_khau_chan_mat_khau_da_lo() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        let session = bearer(&app, "ada@example.com", PASSWORD).await;
        let body = json!({"current_password": PASSWORD, "new_password": BREACHED});
        let (status, _, problem) =
            call(&app, "POST", "/me/password", Some(&session), Some(body)).await;
        assert_refused(status, &problem, "new_password");
        // Mật khẩu cũ vẫn dùng được: không có gì đổi.
        bearer(&app, "ada@example.com", PASSWORD).await;

        let body = json!({"current_password": PASSWORD, "new_password": GOOD});
        let (status, _, changed) =
            call(&app, "POST", "/me/password", Some(&session), Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{changed}");
        bearer(&app, "ada@example.com", GOOD).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dat_lai_mat_khau_chan_mat_khau_da_lo() {
    TestDatabase::run(async |test| {
        let (app, services) = app(test).await;
        let body = json!({"email": "ada@example.com"});
        let (status, _, _) = call(&app, "POST", "/password-reset", None, Some(body)).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        crate::testing::send_resets(&services).await;
        let token = crate::testing::last_token(&services);

        let body = json!({"token": token, "password": BREACHED});
        let (status, _, problem) =
            call(&app, "POST", "/password-reset/confirm", None, Some(body)).await;
        assert_refused(status, &problem, "password");

        // Bị từ chối trước khi dùng token: token vẫn đặt lại được.
        let body = json!({"token": token, "password": GOOD});
        let (status, _, done) =
            call(&app, "POST", "/password-reset/confirm", None, Some(body)).await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{done}");
        bearer(&app, "ada@example.com", GOOD).await;
    })
    .await
    .unwrap();
}
