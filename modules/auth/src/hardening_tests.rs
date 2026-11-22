//! Test hồi quy cho các chỗ siết của `auth` (đăng nhập, đăng ký, nhà cung cấp OIDC). Chạy trên
//! SQLite và Postgres.

use autoxide_kernel::access::Realm;
use autoxide_kernel::event::Actor;
use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use serde_json::json;

use crate::http::test_support::{PASSWORD, app_with, call, call_with};
use crate::testing::services;
use crate::user::NewUser;

/// Sai mật khẩu nhiều lần từ một địa chỉ chỉ khoá cặp email + địa chỉ đó: chủ tài khoản ở địa
/// chỉ khác vẫn đăng nhập được.
#[tokio::test]
async fn khoa_dang_nhap_theo_cap_email_va_dia_chi() {
    TestDatabase::run(async |test| {
        let services = services(test).await;
        let tenant = &services.tenant;
        let input = NewUser {
            email: "ada@example.com".to_owned(),
            name: "Ada".to_owned(),
            password: Some(PASSWORD.to_owned()),
        };
        services.users.create(tenant, Actor::system(), input).await.unwrap();
        let attacker = Some("203.0.113.7".parse().unwrap());
        let owner = Some("198.51.100.9".parse().unwrap());
        let login = async |password: &str, ip| {
            services
                .sessions
                .login_in(tenant, Realm::Staff, "ada@example.com", password, ip, None)
                .await
        };
        for _ in 0..5 {
            assert_eq!(
                login("wrong password!", attacker).await.unwrap_err().code(),
                "auth.invalid_credentials"
            );
        }
        let locked = login(PASSWORD, attacker).await.unwrap_err();
        assert_eq!(locked.code(), "auth.login_throttled");
        login(PASSWORD, owner).await.unwrap();
    })
    .await
    .unwrap();
}

/// Người mua kiểm mật khẩu hiện tại theo đúng loại tài khoản của mình.
#[tokio::test]
async fn nguoi_mua_kiem_mat_khau_hien_tai() {
    TestDatabase::run(async |test| {
        let services = services(test).await;
        let tenant = &services.tenant;
        let input = NewUser {
            email: "buyer@example.com".to_owned(),
            name: "Buyer".to_owned(),
            password: Some(PASSWORD.to_owned()),
        };
        let buyer = services.users.create_customer(tenant, Actor::system(), input).await.unwrap();
        services.sessions.check_password(tenant, buyer.id, PASSWORD).await.unwrap();
        let wrong = services.sessions.check_password(tenant, buyer.id, "not my password").await;
        assert_eq!(wrong.unwrap_err().code(), "auth.wrong_password");
    })
    .await
    .unwrap();
}

/// `POST /registrations` trả lời như nhau dù email đã có tài khoản hay chưa; chủ email đã có nhận
/// thư báo, tài khoản của họ không đổi.
#[tokio::test]
async fn dang_ky_tra_loi_nhu_nhau() {
    TestDatabase::run(async |test| {
        let (app, services) = app_with(test, true).await;
        let mut answers = Vec::new();
        for email in ["bob@example.com", "ada@example.com"] {
            let body =
                json!({"email": email, "name": "Someone", "password": "another good password"});
            let (status, _, answer) = call(&app, "POST", "/registrations", None, Some(body)).await;
            assert_eq!(status, StatusCode::ACCEPTED, "{answer}");
            assert_eq!(answer["data"].as_object().unwrap().keys().collect::<Vec<_>>(), ["email"]);
            answers.push(answer);
        }
        let tenant = &services.tenant;
        assert!(services.users.find_by_email(tenant, "bob@example.com").await.unwrap().is_some());
        let sent = services.mails.sent().iter().map(|m| m.to.clone()).collect::<Vec<_>>();
        assert!(sent.contains(&"ada@example.com".to_owned()), "chủ email đã có nhận thư báo");
        // Mật khẩu của Ada không đổi.
        let body = json!({"email": "ada@example.com", "password": PASSWORD});
        let (status, _, _) = call_with(&app, "POST", "/login", &[], Some(body)).await;
        assert_eq!(status, StatusCode::OK);
        // Đầu vào sai vẫn báo 422 như nhau.
        let body = json!({"email": "ada@example.com", "name": "A", "password": "short"});
        let (status, _, _) = call(&app, "POST", "/registrations", None, Some(body)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}

/// Access token của phiên đã kết thúc bị từ chối ngay, không chờ hết hạn
/// (D-20261121-access-token-revocation): đăng xuất chỉ kết thúc đúng phiên đó; đổi mật khẩu kết thúc
/// mọi phiên cũ, chỉ cặp token mới trả về dùng được. Đổi refresh token vẫn ở trong cùng phiên.
#[tokio::test]
async fn access_token_cua_phien_da_ket_thuc_bi_tu_choi_ngay() {
    TestDatabase::run(async |test| {
        let (app, _services) = app_with(test, false).await;
        let login = async || {
            let body = json!({"email": "ada@example.com", "password": PASSWORD});
            let (status, _, answer) = call(&app, "POST", "/login", None, Some(body)).await;
            assert_eq!(status, StatusCode::OK, "{answer}");
            answer["data"].clone()
        };
        let me = async |tokens: &serde_json::Value| {
            let bearer = format!("Bearer {}", tokens["access_token"].as_str().unwrap());
            let (status, _, answer) = call(&app, "GET", "/me", Some(&bearer), None).await;
            (status, answer["code"].as_str().unwrap_or_default().to_owned())
        };
        let first = login().await;
        let second = login().await;
        assert_eq!(me(&first).await.0, StatusCode::OK);

        let out = json!({"refresh_token": first["refresh_token"]});
        let (status, _, _) = call(&app, "POST", "/logout", None, Some(out)).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert_eq!(me(&first).await, (StatusCode::UNAUTHORIZED, "auth.session_ended".to_owned()));
        assert_eq!(me(&second).await.0, StatusCode::OK, "phiên khác không bị ảnh hưởng");

        // Đổi refresh token: phiên vẫn sống, token cũ và mới của nó đều dùng được tới khi hết hạn.
        let body = json!({"refresh_token": second["refresh_token"]});
        let (status, _, renewed) = call(&app, "POST", "/refresh", None, Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{renewed}");
        let renewed = renewed["data"].clone();
        assert_eq!(me(&second).await.0, StatusCode::OK);
        assert_eq!(me(&renewed).await.0, StatusCode::OK);

        let bearer = format!("Bearer {}", renewed["access_token"].as_str().unwrap());
        let body =
            json!({"current_password": PASSWORD, "new_password": "a brand new long password"});
        let (status, _, changed) =
            call(&app, "POST", "/me/password", Some(&bearer), Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{changed}");
        assert_eq!(me(&renewed).await, (StatusCode::UNAUTHORIZED, "auth.session_ended".to_owned()));
        assert_eq!(me(&changed["data"]).await.0, StatusCode::OK);
    })
    .await
    .unwrap();
}

/// Trường lạ (gõ sai tên) trong thân request bị từ chối 422 `invalid_body`, nêu tên trường, thay vì
/// bị lặng lẽ bỏ qua (D-20261121-api-shape).
#[tokio::test]
async fn truong_la_trong_than_request_bi_tu_choi() {
    TestDatabase::run(async |test| {
        let (app, _services) = app_with(test, false).await;
        let body = json!({"email": "ada@example.com", "pasword": PASSWORD});
        let (status, _, answer) = call(&app, "POST", "/login", None, Some(body)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
        assert_eq!(answer["code"], "invalid_body");
        assert!(answer["detail"].as_str().unwrap().contains("pasword"), "{answer}");
    })
    .await
    .unwrap();
}
