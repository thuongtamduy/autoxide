//! Yêu cầu của giao diện (PR #68 của web): tên nút trong `GET /me/memberships`, phần `auth` của
//! `GET /kernel/client-config`. Cả SQLite và Postgres.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::mail::Mailer;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, OrgTree, TenantContext};
use autoxide_organizations::api::{EffectiveSetting, OrgDirectory, Unit};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use serde_json::json;
use uuid::Uuid;

use crate::user::NewUser;

const PASSWORD: &str = "correct horse battery staple";

/// Cây tổ chức giả: mọi nút là cửa hàng "Store 1".
struct Stores;

#[async_trait::async_trait]
impl OrgDirectory for Stores {
    async fn unit(&self, _: &TenantContext, id: OrgId) -> Result<Option<Unit>, Error> {
        Ok(Some(Unit::new(id, None, "store", "Store 1")))
    }
    async fn root(&self, _: &TenantContext) -> Result<Unit, Error> {
        Err(Error::internal("not used"))
    }
    async fn ancestors(&self, _: &TenantContext, _: OrgId) -> Result<Vec<Unit>, Error> {
        Ok(Vec::new())
    }
    async fn is_within(&self, _: &TenantContext, _: OrgId, _: OrgId) -> Result<bool, Error> {
        Ok(false)
    }
    async fn setting(
        &self,
        _: &TenantContext,
        _: OrgId,
        _: &str,
    ) -> Result<Option<EffectiveSetting>, Error> {
        Ok(None)
    }
}

/// Người chỉ có membership ở một cửa hàng (không có quyền xem cây) vẫn thấy tên cửa hàng; trang
/// đăng nhập biết máy chủ cho đăng nhập bằng link.
#[tokio::test]
async fn ten_nut_trong_membership_cua_toi_va_client_config() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let mailer: Arc<dyn Mailer> = services.mails.clone();
        let app = TestApp::builder(test)
            .setting("AUTOXIDE_AUTH_LOGIN_LINK", "true")
            .provide::<dyn Mailer>(mailer)
            .provide::<dyn OrgDirectory>(Arc::new(Stores))
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let tenant = &services.tenant;
        let write = tenant.database().begin_write().await.unwrap();
        let root = OrgTree::insert_root(&write, DEFAULT_TENANT).await.unwrap();
        let store = OrgId::from_uuid(Uuid::now_v7());
        OrgTree::insert(&write, DEFAULT_TENANT, store, root, 8).await.unwrap();
        write.commit().await.unwrap();

        let input = NewUser {
            email: "clerk@example.com".to_owned(),
            name: "Clerk".to_owned(),
            password: Some(PASSWORD.to_owned()),
        };
        let clerk = services.users.create(tenant, Actor::system(), input).await.unwrap().id;
        let role = crate::role::RoleInput {
            denies: Vec::new(),
            requires_mfa: false,
            name: "clerk".to_owned(),
            description: String::new(),
            permissions: Vec::new(),
            grants: Vec::new(),
        };
        let role = services.roles.create(tenant, Actor::system(), role).await.unwrap();
        let everything = Principal::with_all_permissions(Uuid::now_v7());
        let wanted = vec![(store.as_uuid(), vec![role.id])];
        services.roles.set_memberships(tenant, &everything, clerk, wanted).await.unwrap();
        let body = json!({"email": "clerk@example.com", "password": PASSWORD});
        let login = app.post("/api/v1/auth/login").json(body).send().await.unwrap();
        assert_eq!(login.status(), StatusCode::OK, "{}", login.text());
        let token = login.json()["data"]["access_token"].as_str().unwrap().to_owned();

        let mine = app.get("/api/v1/auth/me/memberships").token(&token).send().await.unwrap();
        assert_eq!(mine.status(), StatusCode::OK, "{}", mine.text());
        assert_eq!(mine.json()["data"][0]["org_name"], "Store 1", "{}", mine.text());
        assert_eq!(mine.json()["data"][0]["org_unit_type"], "store");

        let config = app.get("/api/v1/kernel/client-config").send().await.unwrap();
        assert_eq!(
            config.json()["data"]["auth"],
            json!({"login_link": true, "passkeys": false, "passkey_rp_id": "", "self_registration": false, "support_access_max_hours": 8}),
            "{}",
            config.text()
        );
    })
    .await
    .unwrap();
}
