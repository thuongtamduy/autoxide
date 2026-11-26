//! Đăng nhập OIDC lần đầu theo cài đặt tự đăng ký của khách (`auth.staff_registration`): khách đóng (máy chủ tắt,
//! chưa đặt) thì không tạo tài khoản; khách tự mở thì tạo. Chạy trên cả SQLite và Postgres.

use std::sync::Arc;

use autoxide_kernel::event::Actor;
use autoxide_kernel::mail::Mailer;
use autoxide_organizations::api::OrgDirectory;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;

use super::tests::{claims, fake_server, oidc_body, provider, sign_in, token};
use crate::http::test_support::PASSWORD;
use crate::registration_tests::StaffRegistration;
use crate::user::NewUser;

#[tokio::test]
async fn dang_nhap_lan_dau_theo_cai_dat_tu_dang_ky_cua_khach() {
    TestDatabase::run(async |test| {
        let fake = fake_server().await;
        let services = crate::testing::services(test).await;
        let orgs = Arc::new(StaffRegistration::default());
        let provided: Arc<dyn OrgDirectory> = orgs.clone();
        let mailer: Arc<dyn Mailer> = services.mails.clone();
        let app = TestApp::builder(test)
            .setting("AUTOXIDE_AUTH_TOKEN_SECRET", crate::testing::TOKEN_SECRET)
            .setting("AUTOXIDE_AUTH_OIDC", "true")
            .setting("AUTOXIDE_AUTH_OIDC_ALLOW_PRIVATE_NETWORKS", "true")
            .provide::<dyn Mailer>(mailer)
            .provide::<dyn OrgDirectory>(provided)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let root = NewUser {
            email: "root@example.com".to_owned(),
            name: "Root".to_owned(),
            password: Some(PASSWORD.to_owned()),
        };
        services
            .users
            .create_with_roles(&services.tenant, Actor::system(), root, true)
            .await
            .unwrap();
        let admin = token(&app, "root@example.com").await;
        provider(&app, &admin, oidc_body(&fake, "open", true)).await;
        let grace = || claims(&fake, "open", "s-1", "grace@example.com", true);

        // Máy chủ tắt tự đăng ký, khách chưa đặt: không có role mặc định thì đóng.
        let (status, body) = sign_in(&app, &fake, "open", grace()).await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("auth.oidc_signup_closed"))
        );
        // Khách tự mở: tài khoản mới được tạo.
        orgs.set(services.tenant.id(), true);
        let (status, body) = sign_in(&app, &fake, "open", grace()).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    })
    .await
    .unwrap();
}
