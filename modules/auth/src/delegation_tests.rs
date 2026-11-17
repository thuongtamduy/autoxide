//! Uỷ quyền theo cây khách hàng (bước 34 mục 10, D-310), qua REST với module `auth` thật trên
//! hai khách cha – con (cả SQLite và Postgres).

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

struct Chain {
    app: TestApp,
    parent: TenantContext,
    child: TenantContext,
    /// Token của quản trị viên khách cha và của người giữ role `support`.
    admin: String,
    agent: String,
    role_id: String,
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

async fn sign_in(app: &TestApp, slug: &str, email: &str) -> String {
    let body = json!({"email": email, "password": PASSWORD});
    let response = app
        .post("/api/v1/auth/login")
        .header("autoxide-tenant", slug)
        .json(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "{}", response.json());
    response.json()["data"]["access_token"].as_str().unwrap().to_owned()
}

async fn chain(test: &TestDatabase) -> (Chain, crate::testing::Services) {
    let services = crate::testing::services(test).await;
    let mailer: Arc<dyn Mailer> = services.mails.clone();
    let app = TestApp::builder(test)
        .tenant("parent")
        .tenant("child")
        .tenant("stranger")
        .provide::<dyn Mailer>(mailer)
        .module(crate::module())
        .build()
        .await
        .unwrap();
    let parent = app.tenant("parent").await.unwrap();
    let child = app.tenant("child").await.unwrap();
    // `child` là con của `parent` (bảng khách của kernel ở database điều khiển).
    let control = app.database();
    control
        .write()
        .execute_raw(Statement::from_sql_and_values(
            control.write().get_database_backend(),
            "UPDATE kernel_tenants SET parent_id = $1 WHERE id = $2",
            [parent.id().as_uuid().into(), child.id().as_uuid().into()],
        ))
        .await
        .unwrap();
    let admin_id = person(&services, &parent, "boss@parent.example").await;
    let write = parent.database().begin_write().await.unwrap();
    crate::role::grant_admin(&write, &parent, admin_id).await.unwrap();
    write.commit().await.unwrap();
    let agent_id = person(&services, &parent, "agent@parent.example").await;
    let role = RoleInput {
        requires_mfa: false,
        name: "support".to_owned(),
        description: String::new(),
        permissions: vec!["auth.customers.read".to_owned(), "auth.customers.manage".to_owned()],
        grants: Vec::new(),
    };
    let role = services.roles.create(&parent, Actor::system(), role).await.unwrap();
    services.roles.set_user_roles(&parent, Actor::system(), agent_id, vec![role.id]).await.unwrap();
    let admin = sign_in(&app, "parent", "boss@parent.example").await;
    let agent = sign_in(&app, "parent", "agent@parent.example").await;
    let chain = Chain { app, parent, child, admin, agent, role_id: role.id.to_string() };
    (chain, services)
}

async fn call(
    chain: &Chain,
    method: &str,
    path: &str,
    token: &str,
    tenant: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let method = axum::http::Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = chain.app.request(method, &format!("/api/v1/auth{path}")).token(token);
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

#[tokio::test]
async fn uy_quyen_cha_vao_con_va_audit_hai_phia() {
    TestDatabase::run(async |test| {
        let (chain, services) = chain(test).await;
        let buyer = NewUser { email: "buyer@example.com".to_owned(), name: "Buyer".to_owned(), password: None };
        let buyer = services.users.create_customer(&chain.child, Actor::system(), buyer).await.unwrap();

        // Chưa uỷ quyền: vào khách con là sai khách.
        let (status, refused) = call(&chain, "GET", "/customers", &chain.agent, Some("child"), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");

        let body = json!({"child_tenant_id": chain.child.id().as_uuid(), "role_id": chain.role_id, "reason": "Run the shop's customer care"});
        let (status, granted) = call(&chain, "POST", "/delegations", &chain.admin, None, Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{granted}");
        let delegation = granted["data"]["id"].as_str().unwrap().to_owned();

        let (status, listed) = call(&chain, "GET", "/customers", &chain.agent, Some("child"), None).await;
        assert_eq!(status, StatusCode::OK, "{listed}");
        assert_eq!(listed["data"][0]["id"], buyer.id.to_string());
        // Chỉ quyền của role: không xem được nhân sự của khách con.
        let (status, _) = call(&chain, "GET", "/users", &chain.agent, Some("child"), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (_, tenants) = call(&chain, "GET", "/me/tenants", &chain.agent, None, None).await;
        let access: Vec<(&str, &str)> = tenants["data"].as_array().unwrap().iter().map(|t| (t["slug"].as_str().unwrap(), t["access"].as_str().unwrap())).collect();
        assert_eq!(access, [("parent", "direct"), ("child", "delegation")]);

        // Thay đổi ở khách con mang đường vào (audit của con thấy); việc cấp ghi ở khách cha.
        let path = format!("/customers/{}", buyer.id);
        let (status, _) = call(&chain, "PATCH", &path, &chain.agent, Some("child"), Some(json!({"disabled": true}))).await;
        assert_eq!(status, StatusCode::OK);
        let rows = chain.child.database().read().query_all_raw(Statement::from_sql_and_values(
            chain.child.database().read().get_database_backend(),
            "SELECT actor_access, actor_access_id FROM auth_events WHERE tenant_id = $1 AND name = 'auth.user_updated'",
            [chain.child.id().as_uuid().into()],
        )).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].try_get::<Option<String>>("", "actor_access").unwrap().as_deref(), Some("delegation"));
        assert_eq!(rows[0].try_get::<Option<uuid::Uuid>>("", "actor_access_id").unwrap().map(|id| id.to_string()), Some(delegation.clone()));
        let granted_rows = chain.parent.database().read().query_all_raw(Statement::from_sql_and_values(
            chain.parent.database().read().get_database_backend(),
            "SELECT id FROM auth_events WHERE tenant_id = $1 AND name = 'auth.delegation_granted'",
            [chain.parent.id().as_uuid().into()],
        )).await.unwrap();
        assert_eq!(granted_rows.len(), 1);

        // Khách con thấy uỷ quyền nhận được.
        let child_admin = person(&services, &chain.child, "boss@child.example").await;
        let write = chain.child.database().begin_write().await.unwrap();
        crate::role::grant_admin(&write, &chain.child, child_admin).await.unwrap();
        write.commit().await.unwrap();
        let child_boss = sign_in(&chain.app, "child", "boss@child.example").await;
        let (_, incoming) = call(&chain, "GET", "/delegations?direction=incoming", &child_boss, None, None).await;
        assert_eq!(incoming["data"][0]["id"], delegation.as_str(), "{incoming}");

        // Thu hồi: request kế bị từ chối.
        let (status, _) = call(&chain, "DELETE", &format!("/delegations/{delegation}"), &chain.admin, None, None).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = call(&chain, "GET", "/customers", &chain.agent, Some("child"), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn uy_quyen_khong_cho_khach_khong_phai_con() {
    TestDatabase::run(async |test| {
        let (chain, _) = chain(test).await;
        let stranger = chain.app.tenant("stranger").await.unwrap();
        let body = json!({"child_tenant_id": stranger.id().as_uuid(), "role_id": chain.role_id, "reason": "No"});
        let (status, refused) = call(&chain, "POST", "/delegations", &chain.admin, None, Some(body)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
        assert_eq!(refused["errors"][0]["code"], "auth.delegation_not_descendant");
        // Người không có `auth.delegations.manage` không cấp được.
        let body = json!({"child_tenant_id": chain.child.id().as_uuid(), "role_id": chain.role_id, "reason": "Me"});
        let (status, _) = call(&chain, "POST", "/delegations", &chain.agent, None, Some(body)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await
    .unwrap();
}

/// Khách cha xem danh sách khách con để chọn khi uỷ quyền: thấy khách con, không thấy khách lạ;
/// người không có `auth.delegations.manage` bị chặn.
#[tokio::test]
async fn khach_cha_xem_danh_sach_khach_con() {
    TestDatabase::run(async |test| {
        let (chain, _) = chain(test).await;
        let (status, listed) =
            call(&chain, "GET", "/delegations/tenants", &chain.admin, None, None).await;
        assert_eq!(status, StatusCode::OK, "{listed}");
        let rows = listed["data"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "{listed}");
        assert_eq!(rows[0]["id"], chain.child.id().as_uuid().to_string());
        assert_eq!(rows[0]["parent_id"], chain.parent.id().as_uuid().to_string());
        assert_eq!(rows[0]["slug"], "child");
        let (status, _) =
            call(&chain, "GET", "/delegations/tenants", &chain.agent, None, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await
    .unwrap();
}
