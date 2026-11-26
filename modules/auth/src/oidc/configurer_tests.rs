//! Vai "người cấu hình" của nhà cung cấp đăng nhập ngoài (quyết định có tự gộp vào tài khoản có sẵn
//! không) không sống lâu hơn quyền của người đó (D-20261125-durable-grants): mất `auth.oidc.manage`
//! hay bị khoá hết cửa vào thì nhà cung cấp ngừng tự gộp. Cả SQLite và Postgres.

use autoxide_kernel::event::Actor;
use autoxide_kernel::time::Timestamp;
use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;

use super::tests::{app, claims, fake_server, oidc_body, provider, sign_in, token};
use crate::http::test_support::PASSWORD;
use crate::user::NewUser;

#[tokio::test]
async fn nguoi_cau_hinh_bi_khoa_het_cua_vao_thi_nha_cung_cap_khong_tu_gop() {
    // Tài khoản cắm sẵn P tạm có `auth.oidc.manage`, dựng nhà cung cấp với issuer kẻ tấn công
    // kiểm soát; sau đó P bị khoá hết cửa vào (đổi email qua SCIM, "không phải tôi"…) rồi được trao
    // toàn quyền. Issuer đó khai email đã xác minh của admin: không được gộp vào admin.
    TestDatabase::run(async |test| {
        let fake = fake_server().await;
        let (app, services, _) = app(test, true).await;
        let tenant = services.tenant.clone();
        let input = crate::role::RoleInput {
            denies: Vec::new(),
            requires_mfa: false,
            name: "sso".to_owned(),
            description: String::new(),
            permissions: vec!["auth.oidc.manage".to_owned()],
            grants: Vec::new(),
        };
        let sso = services.roles.create(&tenant, Actor::system(), input).await.unwrap();
        let planted = NewUser {
            email: "planted@example.com".to_owned(),
            name: "P".to_owned(),
            password: Some(PASSWORD.to_owned()),
        };
        let planted = services.users.create(&tenant, Actor::system(), planted).await.unwrap();
        services
            .roles
            .set_user_roles(&tenant, Actor::system(), planted.id, vec![sso.id])
            .await
            .unwrap();
        let planted_token = token(&app, "planted@example.com").await;
        provider(&app, &planted_token, oidc_body(&fake, "evil", false)).await;
        let root =
            services.users.find_by_email(&tenant, "root@example.com").await.unwrap().unwrap();
        let transaction = tenant.database().begin_write().await.unwrap();
        services
            .users
            .apply_verified_email(&tenant, &transaction, root.id, &root.email, Timestamp::now())
            .await
            .unwrap();
        transaction.commit().await.unwrap();

        // Khoá hết cửa vào của P (như khi SCIM đổi email), rồi P được trao toàn quyền.
        let transaction = tenant.database().begin_write().await.unwrap();
        services
            .users
            .lock_out_in(&tenant, &transaction, Actor::system(), planted.id)
            .await
            .unwrap();
        transaction.commit().await.unwrap();
        crate::role::grant_admin(services.database.write(), &tenant, planted.id).await.unwrap();

        let (status, body) =
            sign_in(&app, &fake, "evil", claims(&fake, "evil", "s-root", "root@example.com", true))
                .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["code"], "auth.oidc_link_required", "{body}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nguoi_cau_hinh_mat_quyen_quan_ly_oidc_thi_nha_cung_cap_khong_tu_gop() {
    // Người cấu hình còn role `editor` nhưng mất `auth.oidc.manage`: không gộp vào tài khoản `editor`.
    TestDatabase::run(async |test| {
        let fake = fake_server().await;
        let (app, services, _) = app(test, true).await;
        let tenant = services.tenant.clone();
        let role = |name: &str, permissions: &[&str]| crate::role::RoleInput {
            denies: Vec::new(),
            requires_mfa: false,
            name: name.to_owned(),
            description: String::new(),
            permissions: permissions.iter().map(|permission| (*permission).to_owned()).collect(),
            grants: Vec::new(),
        };
        let sso = services
            .roles
            .create(&tenant, Actor::system(), role("sso", &["auth.oidc.manage"]))
            .await
            .unwrap();
        let editor = services
            .roles
            .create(&tenant, Actor::system(), role("editor", &["notes.write"]))
            .await
            .unwrap();
        let manager = NewUser {
            email: "sso@example.com".to_owned(),
            name: "SSO".to_owned(),
            password: Some(PASSWORD.to_owned()),
        };
        let manager = services.users.create(&tenant, Actor::system(), manager).await.unwrap();
        services
            .roles
            .set_user_roles(&tenant, Actor::system(), manager.id, vec![sso.id, editor.id])
            .await
            .unwrap();
        let manager_token = token(&app, "sso@example.com").await;
        provider(&app, &manager_token, oidc_body(&fake, "idp", false)).await;
        let ada = services.users.find_by_email(&tenant, "ada@example.com").await.unwrap().unwrap();
        services
            .roles
            .set_user_roles(&tenant, Actor::system(), ada.id, vec![editor.id])
            .await
            .unwrap();
        let transaction = tenant.database().begin_write().await.unwrap();
        services
            .users
            .apply_verified_email(&tenant, &transaction, ada.id, &ada.email, Timestamp::now())
            .await
            .unwrap();
        transaction.commit().await.unwrap();

        services
            .roles
            .set_user_roles(&tenant, Actor::system(), manager.id, vec![editor.id])
            .await
            .unwrap();
        let (status, body) =
            sign_in(&app, &fake, "idp", claims(&fake, "idp", "s-ada", "ada@example.com", true))
                .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["code"], "auth.oidc_link_required", "{body}");
    })
    .await
    .unwrap();
}

/// Role `admin` của khách (id dạng JSON).
async fn admin_role(app: &autoxide_testkit::TestApp, admin: &str) -> serde_json::Value {
    let roles = app.get("/api/v1/auth/roles").token(admin).send().await.unwrap().json();
    roles["data"].as_array().unwrap().iter().find(|role| role["name"] == "admin").unwrap()["id"]
        .clone()
}

#[tokio::test]
async fn sua_nha_cung_cap_co_role_mac_dinh_phai_trao_duoc_role_do() {
    // Mọi lần sửa nhà cung cấp còn role mặc định, không chỉ khi đổi trường mặc định.
    TestDatabase::run(async |test| {
        let fake = fake_server().await;
        let (app, services, admin) = app(test, true).await;
        let tenant = services.tenant.clone();
        let mut body = oidc_body(&fake, "staff", true);
        body["default_role_id"] = admin_role(&app, &admin).await;
        let created = provider(&app, &admin, body).await;
        let input = crate::role::RoleInput {
            denies: Vec::new(),
            requires_mfa: false,
            name: "sso".to_owned(),
            description: String::new(),
            permissions: vec!["auth.oidc.manage".to_owned()],
            grants: Vec::new(),
        };
        let sso = services.roles.create(&tenant, Actor::system(), input).await.unwrap();
        let manager = NewUser {
            email: "sso@example.com".to_owned(),
            name: "SSO".to_owned(),
            password: Some(PASSWORD.to_owned()),
        };
        let manager = services.users.create(&tenant, Actor::system(), manager).await.unwrap();
        services
            .roles
            .set_user_roles(&tenant, Actor::system(), manager.id, vec![sso.id])
            .await
            .unwrap();
        let manager_token = token(&app, "sso@example.com").await;
        let path = format!("/api/v1/auth/oidc-providers/{}", created["id"].as_str().unwrap());
        let response = app
            .patch(&path)
            .token(&manager_token)
            .json(serde_json::json!({"allowed_email_domains": ["example.com"]}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY, "{}", response.json());
        assert_eq!(
            response.json()["errors"][0]["code"],
            "more_than_you_have",
            "{}",
            response.json()
        );
        let message = response.json()["errors"][0]["detail"].as_str().unwrap().to_owned();
        assert!(message.contains("remove the default role"), "{message}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tu_dang_ky_voi_role_mac_dinh_kiem_lai_nguoi_cau_hinh() {
    // Người cấu hình (admin thứ hai) bị hạ quyền: tự đăng ký không còn cấp role mặc định — từ chối.
    TestDatabase::run(async |test| {
        let fake = fake_server().await;
        let (app, services, admin) = app(test, true).await;
        let tenant = services.tenant.clone();
        let second = NewUser {
            email: "second@example.com".to_owned(),
            name: "Second".to_owned(),
            password: Some(PASSWORD.to_owned()),
        };
        let second =
            services.users.create_with_roles(&tenant, Actor::system(), second, true).await.unwrap();
        let second_token = token(&app, "second@example.com").await;
        let mut body = oidc_body(&fake, "staff", true);
        body["default_role_id"] = admin_role(&app, &admin).await;
        provider(&app, &second_token, body).await;
        let (status, body) =
            sign_in(&app, &fake, "staff", claims(&fake, "staff", "s-1", "new1@example.com", true))
                .await;
        assert_eq!(status, StatusCode::OK, "{body}");

        services.roles.set_user_roles(&tenant, Actor::system(), second.id, vec![]).await.unwrap();
        let (status, body) =
            sign_in(&app, &fake, "staff", claims(&fake, "staff", "s-2", "new2@example.com", true))
                .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(body["code"], "auth.oidc_default_role_unavailable", "{body}");
        assert!(services.users.find_by_email(&tenant, "new2@example.com").await.unwrap().is_none());
    })
    .await
    .unwrap();
}

/// Gói giả: không còn chỗ nhân sự nào (`auth.members`), mọi thứ khác không giới hạn.
struct NoSeats;

#[async_trait::async_trait]
impl autoxide_kernel::entitlement::Entitlements for NoSeats {
    async fn snapshot(
        &self,
        _: autoxide_kernel::tenancy::TenantId,
    ) -> Result<
        std::sync::Arc<autoxide_kernel::entitlement::TenantEntitlements>,
        autoxide_kernel::error::Error,
    > {
        Ok(std::sync::Arc::new(autoxide_kernel::entitlement::TenantEntitlements {
            limits: [("auth.members".to_owned(), Some(0))].into(),
            features: ["auth.oidc".to_owned()].into(),
            ..autoxide_kernel::entitlement::TenantEntitlements::unrestricted()
        }))
    }

    async fn reserve(
        &self,
        _: autoxide_kernel::tenancy::TenantId,
        limit: &autoxide_kernel::entitlement::Limit,
        _: i64,
        _: &str,
    ) -> Result<(), autoxide_kernel::error::Error> {
        if limit.key() == "auth.members" {
            return Err(autoxide_kernel::entitlement::limit_exceeded("auth.members", 0, 0));
        }
        Ok(())
    }

    async fn confirm(
        &self,
        _: autoxide_kernel::tenancy::TenantId,
        _: &autoxide_kernel::entitlement::Limit,
        _: &str,
    ) -> Result<(), autoxide_kernel::error::Error> {
        Ok(())
    }

    async fn release(
        &self,
        _: autoxide_kernel::tenancy::TenantId,
        _: &autoxide_kernel::entitlement::Limit,
        _: &str,
    ) -> Result<(), autoxide_kernel::error::Error> {
        Ok(())
    }

    async fn record_usage(
        &self,
        _: autoxide_kernel::tenancy::TenantId,
        _: &autoxide_kernel::entitlement::Limit,
        _: i64,
        _: &str,
    ) -> Result<(), autoxide_kernel::error::Error> {
        Ok(())
    }
}

#[tokio::test]
async fn tu_dang_ky_loi_giua_chung_khong_de_lai_tai_khoan() {
    // Gói hết chỗ nhân sự: tự đăng ký với role mặc định bị từ chối và không để lại tài khoản không
    // role, không liên kết. Khách riêng `alpha` (khách mặc định được miễn hạn mức).
    TestDatabase::run(async |test| {
        use super::tests::{approve, callback, start};
        let fake = fake_server().await;
        let services = crate::testing::services(test).await;
        let mailer: std::sync::Arc<dyn autoxide_kernel::mail::Mailer> = services.mails.clone();
        let plan: std::sync::Arc<dyn autoxide_kernel::entitlement::Entitlements> =
            std::sync::Arc::new(NoSeats);
        let app = autoxide_testkit::TestApp::builder(test)
            .tenant("alpha")
            .setting("AUTOXIDE_AUTH_TOKEN_SECRET", crate::testing::TOKEN_SECRET)
            .setting("AUTOXIDE_AUTH_OIDC", "true")
            .setting("AUTOXIDE_AUTH_OIDC_ALLOW_PRIVATE_NETWORKS", "true")
            .provide::<dyn autoxide_kernel::mail::Mailer>(mailer)
            .provide::<dyn autoxide_kernel::entitlement::Entitlements>(plan)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let tenant = app.tenant("alpha").await.unwrap();
        let boss = NewUser {
            email: "boss@alpha.example".to_owned(),
            name: "Boss".to_owned(),
            password: Some(PASSWORD.to_owned()),
        };
        services.users.create_with_roles(&tenant, Actor::system(), boss, true).await.unwrap();
        let response = app
            .post("/api/v1/auth/login")
            .header("autoxide-tenant", "alpha")
            .json(serde_json::json!({"email": "boss@alpha.example", "password": PASSWORD}))
            .send()
            .await
            .unwrap();
        let admin = response.json()["data"]["access_token"].as_str().unwrap().to_owned();
        let roles = app
            .get("/api/v1/auth/roles")
            .token(&admin)
            .header("autoxide-tenant", "alpha")
            .send()
            .await
            .unwrap()
            .json();
        let mut body = oidc_body(&fake, "staff", true);
        body["default_role_id"] = roles["data"].as_array().unwrap()[0]["id"].clone();
        let created = app
            .post("/api/v1/auth/oidc-providers")
            .token(&admin)
            .header("autoxide-tenant", "alpha")
            .json(body)
            .send()
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());

        let query = start(&app, "staff", Some("alpha")).await;
        let code = approve(&fake, &query, claims(&fake, "staff", "s-1", "new@alpha.example", true));
        let (status, body) = callback(&app, "staff", &code, &query["state"]).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(body["code"], "limit_exceeded", "{body}");
        assert!(
            services.users.find_by_email(&tenant, "new@alpha.example").await.unwrap().is_none()
        );
    })
    .await
    .unwrap();
}
