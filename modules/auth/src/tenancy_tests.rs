//! Module `auth` theo khách (bước 33 B12): cùng email / tên role ở hai khách, role `admin` gieo cho
//! khách mới, secret TOTP lưu mã hoá (upgrade `u0001_seal_totp`), không rò rỉ giữa khách qua REST
//! và MCP.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::event::Actor;
use autoxide_kernel::mail::Mailer;
use autoxide_kernel::migration::{DataUpgrade, UpgradeContext};
use autoxide_kernel::pagination::{Order, PageRequest};
use autoxide_kernel::secret::SecretKey;
use autoxide_kernel::tenancy::{MAIN_PLACEMENT, TenantContext, TenantId, TenantSeed, TenantStatus};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use chrono::{SubsecRound, Utc};
use sea_orm::{ActiveModelTrait, ConnectionTrait, Set, Statement};
use serde_json::json;
use uuid::Uuid;

use crate::api_key::NewApiKey;
use crate::role::{AdminRoleSeed, RoleInput};
use crate::user::NewUser;

const PASSWORD: &str = "correct horse battery staple";

fn new_user(email: &str, password: &str) -> NewUser {
    NewUser { email: email.to_owned(), name: "Ada".to_owned(), password: Some(password.to_owned()) }
}

fn role(name: &str) -> RoleInput {
    RoleInput { name: name.to_owned(), description: String::new(), permissions: Vec::new() }
}

/// Cùng email, cùng tên role ở hai khách đều tạo được; đăng nhập ở khách nào thì tìm tài khoản của
/// khách đó (mật khẩu của tài khoản khách kia sai); trong một khách vẫn bị chặn.
#[tokio::test]
async fn trung_ten_o_hai_khach_duoc_phep() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let a = services.tenant.clone();
        let b = TenantContext::new(
            TenantId::from_uuid(Uuid::now_v7()),
            "other",
            "Other",
            TenantStatus::Active,
            MAIN_PLACEMENT,
            services.database.clone(),
        );
        let in_a =
            services.users.create(&a, Actor::system(), new_user("ada@example.com", PASSWORD));
        let in_a = in_a.await.unwrap();
        let other_password = "another long passphrase";
        let in_b =
            services.users.create(&b, Actor::system(), new_user("ADA@example.com", other_password));
        let in_b = in_b.await.unwrap();
        assert_ne!(in_a.id, in_b.id);
        services.roles.create(&a, Actor::system(), role("editors")).await.unwrap();
        services.roles.create(&b, Actor::system(), role("editors")).await.unwrap();

        // Mỗi khách chỉ thấy người dùng của mình.
        assert_eq!(services.users.get(&b, in_a.id).await.unwrap(), None);
        let page = PageRequest::first(10, Order::Asc);
        let listed = services.users.list(&b, None, None, &page).await.unwrap();
        assert_eq!(listed.data.iter().map(|user| user.id).collect::<Vec<_>>(), [in_b.id]);

        // Đăng nhập ở khách `b`: tài khoản của `b`; mật khẩu của tài khoản `a` thì sai.
        let tokens = services.sessions.login(&b, "ada@example.com", other_password, None);
        let tokens = tokens.await.unwrap().tokens();
        let bearer = format!("Bearer {}", tokens.access_token);
        assert_eq!(services.sessions.authenticate(&b, Some(&bearer)).await.unwrap().id, in_b.id);
        let wrong = services.sessions.login(&b, "ada@example.com", PASSWORD, None).await;
        assert_eq!(wrong.unwrap_err().code(), "auth.invalid_credentials");
        let mine = services.sessions.login(&a, "ada@example.com", PASSWORD, None).await;
        assert!(mine.is_ok());

        // Trong một khách vẫn trùng.
        let again =
            services.users.create(&b, Actor::system(), new_user("ada@example.com", PASSWORD));
        assert_eq!(again.await.unwrap_err().code(), "auth.email_taken");
        let again = services.roles.create(&b, Actor::system(), role("editors")).await;
        assert_eq!(again.unwrap_err().code(), "auth.role_name_taken");
    })
    .await
    .unwrap();
}

/// Secret TOTP bản cũ (chữ thường) được upgrade `u0001_seal_totp` mã hoá: database không còn chữ
/// thường, mã sinh từ secret cũ vẫn đúng; secret mới bật sau này lưu đã mã hoá; `down` trả về chữ
/// thường.
#[tokio::test]
async fn totp_luu_ma_hoa_va_ma_cu_doc_duoc() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let tenant = services.tenant.clone();
        let database = services.database.clone();
        let ada =
            services.users.create(&tenant, Actor::system(), new_user("ada@example.com", PASSWORD));
        let ada = ada.await.unwrap();
        // Như bản trước bước 33: secret base32 lưu nguyên văn, đã xác nhận.
        let plain = "JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP";
        let now = Utc::now().trunc_subsecs(6);
        crate::mfa::entity::totp::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            user_id: Set(ada.id),
            secret: Set(plain.to_owned()),
            confirmed_at: Set(Some(now)),
            last_used_step: Set(None),
            created_at: Set(now),
        }
        .insert(database.write())
        .await
        .unwrap();

        let stored = async || {
            let backend = database.write().get_database_backend();
            let sql = "SELECT secret FROM auth_totp";
            let rows =
                database.write().query_all_raw(Statement::from_string(backend, sql)).await.unwrap();
            rows.iter().map(|row| row.try_get::<String>("", "secret").unwrap()).collect::<Vec<_>>()
        };
        let context = UpgradeContext::new(database.clone(), SecretKey::new(database.clone(), None));
        let upgrade = crate::migration::u0001_seal_totp::SealTotp;
        upgrade.up(&context).await.unwrap();
        let sealed = stored().await;
        // Chạy lại không mã hoá chồng.
        upgrade.up(&context).await.unwrap();
        assert_eq!(stored().await, sealed);
        assert!(sealed[0].starts_with("sealed:v1:"), "{sealed:?}");
        assert!(!sealed[0].contains(plain));

        // Mã của secret cũ vẫn đúng (mở secret đã mã hoá lúc kiểm).
        let code = crate::mfa::service::tests::code_at(
            plain,
            u64::try_from(Utc::now().timestamp()).unwrap(),
        );
        assert!(services.mfa.verify_totp(&tenant, ada.id, &code).await.unwrap());

        // `down` trả về chữ thường để bản cũ đọc được.
        upgrade.down(&context).await.unwrap();
        assert_eq!(stored().await, [plain]);

        // Bật TOTP mới: lưu đã mã hoá ngay.
        let bob =
            services.users.create(&tenant, Actor::system(), new_user("bob@example.com", PASSWORD));
        let bob = bob.await.unwrap();
        let setup = services.mfa.start_totp(&tenant, bob.id, "bob@example.com").await.unwrap();
        let all = stored().await;
        assert!(all.iter().any(|secret| secret.starts_with("sealed:v1:")), "{all:?}");
        assert!(all.iter().all(|secret| !secret.contains(&setup.secret)));
    })
    .await
    .unwrap();
}

/// Khách tạo qua dịch vụ khách của kernel có role `admin` (mọi permission); gieo lại không nhân
/// đôi. Người của khách B không thấy người dùng, role, API key của khách A qua bất kỳ `GET` hay
/// tool MCP chỉ đọc nào (một khách dùng chung database, một khách database riêng); cùng email ở
/// hai khách đều tạo được và đăng nhập ở khách nào thì vào tài khoản của khách đó.
#[tokio::test]
async fn khong_ro_ri_giua_khach() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let mailer: Arc<dyn Mailer> = services.mails.clone();
        let all = [
            "auth.users.read",
            "auth.users.manage",
            "auth.roles.read",
            "auth.roles.manage",
            "auth.api_keys.manage",
            "auth.passkeys.manage",
        ];
        let app = TestApp::builder(test)
            .tenant("a")
            .tenant_dedicated("b")
            .user_in("a", "admin-a", 1, &all)
            .user_in("b", "admin-b", 2, &all)
            .provide::<dyn Mailer>(mailer)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let (a, b) = (app.tenant("a").await.unwrap(), app.tenant("b").await.unwrap());

        // Khách mới có role `admin`; gieo lại không thêm dòng.
        for tenant in [&a, &b] {
            AdminRoleSeed.seed(tenant).await.unwrap();
            let page = PageRequest::first(50, Order::Asc);
            let roles = services.roles.list(tenant, &page).await.unwrap().data;
            let admins: Vec<_> = roles.iter().filter(|role| role.all_permissions).collect();
            assert_eq!(admins.len(), 1, "{roles:?}");
            assert_eq!(admins[0].name, "admin");
        }

        // Người của khách A tạo người dùng, role (qua REST) và API key mang dấu `marker`.
        let marker = "marker-auth-a";
        let email = format!("{marker}@example.com");
        let body = json!({"email": email, "name": marker, "password": PASSWORD});
        let created =
            app.post("/api/v1/auth/users").token("admin-a").json(body).send().await.unwrap();
        assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
        let user_id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        let body =
            json!({"name": marker, "description": marker, "permissions": ["auth.users.read"]});
        let role = app.post("/api/v1/auth/roles").token("admin-a").json(body).send().await.unwrap();
        assert_eq!(role.status(), StatusCode::CREATED, "{}", role.json());
        let role_id = role.json()["data"]["id"].as_str().unwrap().to_owned();
        let owner = Principal::new(user_id.parse().unwrap(), ["auth.users.read".to_owned()])
            .in_tenant(a.id());
        let key = NewApiKey {
            name: marker.to_owned(),
            permissions: vec!["auth.users.read".to_owned()],
            expires_at: None,
            current_password: PASSWORD.to_owned(),
            second_factor: None,
        };
        let key = services.api_keys.create(&a, &owner, key).await.unwrap().key.id.to_string();

        // Cùng email ở hai khách: đều tạo được; đăng nhập theo khách request nêu.
        for (who, password) in [("admin-a", PASSWORD), ("admin-b", "another long passphrase")] {
            let body =
                json!({"email": "shared@example.com", "name": "Shared", "password": password});
            let made = app.post("/api/v1/auth/users").token(who).json(body).send().await.unwrap();
            assert_eq!(made.status(), StatusCode::CREATED, "{}", made.json());
        }
        let body = json!({"email": "shared@example.com", "name": "Again"});
        let again =
            app.post("/api/v1/auth/users").token("admin-a").json(body).send().await.unwrap();
        assert_eq!(again.status(), StatusCode::CONFLICT, "{}", again.json());
        let login = async |tenant: &str, password: &str| {
            let body = json!({"email": "shared@example.com", "password": password});
            let request = app.post("/api/v1/auth/login").header("autoxide-tenant", tenant);
            request.json(body).send().await.unwrap().status()
        };
        assert_eq!(login("a", PASSWORD).await, StatusCode::OK);
        assert_eq!(login("b", PASSWORD).await, StatusCode::UNAUTHORIZED);
        assert_eq!(login("b", "another long passphrase").await, StatusCode::OK);

        // Người của khách B không thấy gì của khách A.
        let missing = app.get(&format!("/api/v1/auth/users/{user_id}")).token("admin-b");
        assert_eq!(missing.send().await.unwrap().status(), StatusCode::NOT_FOUND);
        let seeded = [("id", user_id), ("id", role_id), ("id", key)];
        let violations =
            autoxide_testkit::leak_check(&app, "admin-b", marker, &seeded).await.unwrap();
        assert_eq!(violations, Vec::<String>::new());
    })
    .await
    .unwrap();
}
