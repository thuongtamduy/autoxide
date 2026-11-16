//! Quyền vào hỗ trợ của nhân sự nền tảng (bước 34 mục 13, D-190, D-211, D-313), qua REST với module
//! `auth` thật (khách nền tảng + một khách), cả SQLite và Postgres.

use std::sync::Arc;

use autoxide_kernel::event::Actor;
use autoxide_kernel::mail::Mailer;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use sea_orm::{ConnectionTrait, Statement};
use serde_json::{Value, json};

use crate::role::RoleInput;
use crate::user::NewUser;

const PASSWORD: &str = "correct horse battery staple";

struct Setup {
    app: TestApp,
    shop: TenantContext,
    platform_admin: String,
    shop_admin: String,
    agent: String,
    agent_id: uuid::Uuid,
    role_id: uuid::Uuid,
}

async fn person(
    services: &crate::testing::Services,
    tenant: &TenantContext,
    email: &str,
) -> uuid::Uuid {
    let input = NewUser {
        email: email.to_owned(),
        name: email.to_owned(),
        password: Some(PASSWORD.to_owned()),
    };
    services.users.create(tenant, Actor::system(), input).await.unwrap().id
}

async fn sign_in(app: &TestApp, slug: Option<&str>, email: &str) -> String {
    let body = json!({"email": email, "password": PASSWORD});
    let mut request = app.post("/api/v1/auth/login");
    if let Some(slug) = slug {
        request = request.header("autoxide-tenant", slug);
    }
    let response = request.json(body).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK, "{}", response.json());
    response.json()["data"]["access_token"].as_str().unwrap().to_owned()
}

async fn setup(test: &TestDatabase) -> (Setup, crate::testing::Services) {
    let services = crate::testing::services(test).await;
    let mailer: Arc<dyn Mailer> = services.mails.clone();
    let app = TestApp::builder(test)
        .tenant("shop")
        .provide::<dyn Mailer>(mailer)
        .module(crate::module())
        .build()
        .await
        .unwrap();
    let platform = services.tenant.clone();
    let shop = app.tenant("shop").await.unwrap();
    let boss = person(&services, &platform, "boss@platform.example").await;
    let agent_id = person(&services, &platform, "agent@platform.example").await;
    let owner = person(&services, &shop, "owner@shop.example").await;
    for (tenant, user) in [(&platform, boss), (&shop, owner)] {
        let write = tenant.database().begin_write().await.unwrap();
        crate::role::grant_admin(&write, tenant, user).await.unwrap();
        write.commit().await.unwrap();
    }
    let role = RoleInput {
        requires_mfa: false,
        name: "support".to_owned(),
        description: String::new(),
        permissions: vec!["auth.customers.read".to_owned()],
        grants: Vec::new(),
    };
    let role_id = services.roles.create(&platform, Actor::system(), role).await.unwrap().id;
    let setup = Setup {
        platform_admin: sign_in(&app, Some("default"), "boss@platform.example").await,
        shop_admin: sign_in(&app, Some("shop"), "owner@shop.example").await,
        agent: sign_in(&app, Some("default"), "agent@platform.example").await,
        app,
        shop,
        agent_id,
        role_id,
    };
    (setup, services)
}

async fn call(
    setup: &Setup,
    method: &str,
    path: &str,
    token: &str,
    tenant: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let method = axum::http::Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = setup.app.request(method, &format!("/api/v1/auth{path}")).token(token);
    if let Some(tenant) = tenant {
        request = request.header("autoxide-tenant", tenant);
    }
    if let Some(body) = body {
        request = request.json(body);
    }
    let response = request.send().await.unwrap();
    let status = response.status();
    (status, if response.body().is_empty() { Value::Null } else { response.json() })
}

fn grant_body(setup: &Setup, basis: &str, hours: u32) -> Value {
    json!({
        "tenant_id": setup.shop.id().as_uuid(),
        "staff_user_id": setup.agent_id,
        "role_id": setup.role_id,
        "basis": basis,
        "reason": "Customer asked for help with their accounts",
        "legal_reference": if basis == "legal_request" { json!("123/QĐ-CA") } else { Value::Null },
        "hours": hours,
    })
}

#[tokio::test]
async fn ho_tro_het_han_thi_mat_quyen() {
    TestDatabase::run(async |test| {
        let (setup, _) = setup(test).await;
        // Nhân sự nền tảng không vào được dữ liệu khách nếu không có lần cấp.
        let (status, _) = call(&setup, "GET", "/customers", &setup.agent, Some("shop"), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, too_long) = call(
            &setup,
            "POST",
            "/support-grants",
            &setup.platform_admin,
            None,
            Some(grant_body(&setup, "support", 99)),
        )
        .await;
        assert_eq!(
            (status, too_long["code"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("auth.support_duration_exceeded"))
        );
        let (status, granted) = call(
            &setup,
            "POST",
            "/support-grants",
            &setup.platform_admin,
            None,
            Some(grant_body(&setup, "support", 2)),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{granted}");
        let id = granted["data"]["id"].as_str().unwrap().to_owned();

        let (status, listed) =
            call(&setup, "GET", "/customers", &setup.agent, Some("shop"), None).await;
        assert_eq!(status, StatusCode::OK, "{listed}");
        let (_, tenants) = call(&setup, "GET", "/me/tenants", &setup.agent, None, None).await;
        assert_eq!(tenants["data"][1]["access"], "support", "{tenants}");

        // Khách thấy lần cấp và lần vào đầu, trong audit của mình.
        let (_, seen) = call(&setup, "GET", "/support-access", &setup.shop_admin, None, None).await;
        assert_eq!(seen["data"][0]["id"], id.as_str(), "{seen}");
        assert!(seen["data"][0]["first_used_at"].is_string());
        let names: Vec<String> = setup
            .shop
            .database()
            .read()
            .query_all_raw(Statement::from_sql_and_values(
                setup.shop.database().read().get_database_backend(),
                "SELECT name FROM auth_events WHERE tenant_id = $1 ORDER BY id",
                [setup.shop.id().as_uuid().into()],
            ))
            .await
            .unwrap()
            .iter()
            .map(|row| row.try_get::<String>("", "name").unwrap())
            .collect();
        assert!(names.contains(&"auth.support_access_granted".to_owned()), "{names:?}");
        assert!(names.contains(&"auth.support_access_started".to_owned()), "{names:?}");

        // Hết hạn: request kế bị từ chối.
        setup
            .app
            .database()
            .write()
            .execute_raw(Statement::from_sql_and_values(
                setup.app.database().write().get_database_backend(),
                "UPDATE auth_support_grant SET expires_at = $1 WHERE id = $2",
                [
                    (chrono::Utc::now() - chrono::Duration::minutes(1)).into(),
                    uuid::Uuid::parse_str(&id).unwrap().into(),
                ],
            ))
            .await
            .unwrap();
        let (status, _) = call(&setup, "GET", "/customers", &setup.agent, Some("shop"), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khach_tu_thu_hoi_tru_yeu_cau_co_tham_quyen() {
    TestDatabase::run(async |test| {
        let (setup, _) = setup(test).await;
        let (_, granted) = call(
            &setup,
            "POST",
            "/support-grants",
            &setup.platform_admin,
            None,
            Some(grant_body(&setup, "support", 2)),
        )
        .await;
        let id = granted["data"]["id"].as_str().unwrap().to_owned();
        let (status, revoked) =
            call(&setup, "DELETE", &format!("/support-access/{id}"), &setup.shop_admin, None, None)
                .await;
        assert_eq!(status, StatusCode::OK, "{revoked}");
        assert_eq!(revoked["data"]["revoked_by_tenant"], true);
        let (status, _) = call(&setup, "GET", "/customers", &setup.agent, Some("shop"), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "thu hồi có hiệu lực ngay");

        let (status, legal) = call(
            &setup,
            "POST",
            "/support-grants",
            &setup.platform_admin,
            None,
            Some(grant_body(&setup, "legal_request", 2)),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{legal}");
        let id = legal["data"]["id"].as_str().unwrap().to_owned();
        let (status, refused) =
            call(&setup, "DELETE", &format!("/support-access/{id}"), &setup.shop_admin, None, None)
                .await;
        assert_eq!(
            (status, refused["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("auth.support_grant_legal_request"))
        );
        // Nền tảng thu hồi được.
        let path = format!("/support-grants/{id}?tenant_id={}", setup.shop.id().as_uuid());
        let (status, _) = call(&setup, "DELETE", &path, &setup.platform_admin, None, None).await;
        assert_eq!(status, StatusCode::OK);
    })
    .await
    .unwrap();
}

/// `OrgDirectory` giả: khách tắt `auth.support_access`.
struct SupportOff;

#[async_trait::async_trait]
impl autoxide_organizations::api::OrgDirectory for SupportOff {
    async fn unit(
        &self,
        _: &TenantContext,
        _: autoxide_kernel::tenancy::OrgId,
    ) -> Result<Option<autoxide_organizations::api::Unit>, autoxide_kernel::error::Error> {
        Ok(None)
    }
    async fn root(
        &self,
        _: &TenantContext,
    ) -> Result<autoxide_organizations::api::Unit, autoxide_kernel::error::Error> {
        Err(autoxide_kernel::error::Error::internal("not used"))
    }
    async fn ancestors(
        &self,
        _: &TenantContext,
        _: autoxide_kernel::tenancy::OrgId,
    ) -> Result<Vec<autoxide_organizations::api::Unit>, autoxide_kernel::error::Error> {
        Ok(Vec::new())
    }
    async fn is_within(
        &self,
        _: &TenantContext,
        _: autoxide_kernel::tenancy::OrgId,
        _: autoxide_kernel::tenancy::OrgId,
    ) -> Result<bool, autoxide_kernel::error::Error> {
        Ok(false)
    }
    async fn setting(
        &self,
        _: &TenantContext,
        id: autoxide_kernel::tenancy::OrgId,
        key: &str,
    ) -> Result<Option<autoxide_organizations::api::EffectiveSetting>, autoxide_kernel::error::Error>
    {
        let value = (key == "auth.support_access").then_some(json!(false));
        Ok(Some(autoxide_organizations::api::EffectiveSetting::new(key, value, Some(id), false)))
    }
}

#[tokio::test]
async fn khach_tat_ho_tro_chi_con_yeu_cau_co_tham_quyen() {
    TestDatabase::run(async |test| {
        let (setup, services) = setup(test).await;
        let support = crate::support::SupportService {
            databases: setup.app.modules().tenancy().unwrap(),
            outbox: autoxide_kernel::event::Outbox::standalone("auth"),
            orgs: Some(Arc::new(SupportOff)),
            max_hours: 8,
        };
        let boss = autoxide_kernel::access::Principal::with_all_permissions(uuid::Uuid::now_v7())
            .in_tenant(services.tenant.id());
        let input = |basis: &str| crate::support::NewSupportGrant {
            tenant_id: setup.shop.id().as_uuid(),
            staff_user_id: setup.agent_id,
            role_id: setup.role_id,
            basis: basis.to_owned(),
            reason: "Investigation of a reported fraud".to_owned(),
            legal_reference: Some("45/YC-CA".to_owned()),
            hours: 2,
        };
        let refused = support.grant(&boss, input("support")).await.unwrap_err();
        assert_eq!(refused.code(), "auth.support_access_disabled");
        support.grant(&boss, input("legal_request")).await.unwrap();
        let agent = autoxide_kernel::access::Principal::new(setup.agent_id, [])
            .in_tenant(services.tenant.id());
        let entered = support.enter(&agent, &setup.shop).await.unwrap().unwrap();
        assert_eq!(entered.access().as_str(), "support");
        assert!(entered.allows("auth.customers.read"));
    })
    .await
    .unwrap();
}
