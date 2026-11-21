//! Rào quyền tối đa cho khách con (bước 54, D-20261119-step54-guardrails), qua REST với module
//! `auth` thật trên ba khách ông – cha – cháu (cả SQLite và Postgres).

use std::sync::Arc;

use autoxide_kernel::event::Actor;
use autoxide_kernel::mail::Mailer;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use sea_orm::ConnectionTrait;
use serde_json::{Value, json};

use crate::user::NewUser;

const PASSWORD: &str = "correct horse battery staple";

async fn admin_of(
    services: &crate::testing::Services,
    app: &TestApp,
    tenant: &TenantContext,
    slug: &str,
) -> String {
    let email = format!("boss@{slug}.example");
    let input =
        NewUser { email: email.clone(), name: email.clone(), password: Some(PASSWORD.to_owned()) };
    let id = services.users.create(tenant, Actor::system(), input).await.unwrap().id;
    let write = tenant.database().begin_write().await.unwrap();
    crate::role::grant_admin(&write, tenant, id).await.unwrap();
    write.commit().await.unwrap();
    let response = app
        .post("/api/v1/auth/login")
        .header("autoxide-tenant", slug)
        .json(json!({"email": email, "password": PASSWORD}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "{}", response.json());
    response.json()["data"]["access_token"].as_str().unwrap().to_owned()
}

async fn call(
    app: &TestApp,
    method: &str,
    path: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let method = axum::http::Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method, &format!("/api/v1/auth{path}")).token(token);
    if let Some(body) = body {
        request = request.json(body);
    }
    let response = request.send().await.unwrap();
    let status = response.status();
    (status, if response.body().is_empty() { Value::Null } else { response.json() })
}

#[tokio::test]
async fn rao_khach_con_chan_ca_admin_va_khach_chau() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let mailer: Arc<dyn Mailer> = services.mails.clone();
        let app = TestApp::builder(test)
            .tenant("grandpa")
            .tenant("parent")
            .tenant("child")
            .provide::<dyn Mailer>(mailer)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let grandpa = app.tenant("grandpa").await.unwrap();
        let parent = app.tenant("parent").await.unwrap();
        let child = app.tenant("child").await.unwrap();
        // Cây khách: grandpa → parent → child (bảng khách của kernel ở database điều khiển).
        let control = app.database();
        for (up, down) in [(&grandpa, &parent), (&parent, &child)] {
            control
                .write()
                .execute_raw(autoxide_kernel::database::statement(
                    control.write().get_database_backend(),
                    "UPDATE kernel_tenants SET parent_id = $1 WHERE id = $2",
                    [up.id().as_uuid().into(), down.id().as_uuid().into()],
                ))
                .await
                .unwrap();
        }
        let old = admin_of(&services, &app, &grandpa, "grandpa").await;
        let mid = admin_of(&services, &app, &parent, "parent").await;
        let young = admin_of(&services, &app, &child, "child").await;
        let rail = |tenant: &TenantContext| format!("/guardrails/{}", tenant.id().as_uuid());

        // Chưa có rào: admin của khách cháu làm được mọi việc.
        let (status, _) = call(&app, "GET", "/users", &young, None).await;
        assert_eq!(status, StatusCode::OK);

        // Ông rào cha: cháu cũng bị rào, kể cả admin.
        let body = json!({"max_permissions": ["auth.roles.read", "auth.users.read"]});
        let (status, set) = call(&app, "PUT", &rail(&parent), &old, Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{set}");
        assert_eq!(set["data"]["max_permissions"], json!(["auth.roles.read", "auth.users.read"]));
        let (status, _) = call(&app, "GET", "/roles", &young, None).await;
        assert_eq!(status, StatusCode::OK);
        let role = json!({"name": "extra", "permissions": []});
        let (status, refused) = call(&app, "POST", "/roles", &young, Some(role)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
        assert_eq!(refused["code"], "permission_denied");
        assert_eq!(refused["reason"], "outside_guardrail");

        // Cha rào thêm cháu: các rào giao nhau; cha không nới được rào của ông.
        let narrow = json!({"max_permissions": ["auth.roles.read", "auth.roles.manage"]});
        let (status, _) = call(&app, "PUT", &rail(&child), &mid, Some(narrow)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "admin của cha cũng bị rào của ông: thiếu quyền đặt rào");
        // Ông cho cha quyền đặt rào, rồi cha rào cháu.
        let body = json!({"max_permissions": ["auth.roles.read", "auth.users.read", "auth.guardrails.manage"]});
        let (status, _) = call(&app, "PUT", &rail(&parent), &old, Some(body)).await;
        assert_eq!(status, StatusCode::OK);
        let narrow = json!({"max_permissions": ["auth.roles.read", "auth.roles.manage"]});
        let (status, set) = call(&app, "PUT", &rail(&child), &mid, Some(narrow)).await;
        assert_eq!(status, StatusCode::OK, "{set}");
        let (status, _) = call(&app, "GET", "/roles", &young, None).await;
        assert_eq!(status, StatusCode::OK, "cả hai rào cho phép");
        let (status, users) = call(&app, "GET", "/users", &young, None).await;
        assert_eq!((status, users["reason"].as_str()), (StatusCode::FORBIDDEN, Some("outside_guardrail")));
        let role = json!({"name": "extra", "permissions": []});
        let (status, _) = call(&app, "POST", "/roles", &young, Some(role)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "cha cho `auth.roles.manage` nhưng ông không cho");

        // Cháu xem rào đang áp cho mình.
        let (status, applied) = call(&app, "GET", "/guardrails/applied", &young, None).await;
        assert_eq!(status, StatusCode::OK, "{applied}");
        assert_eq!(applied["data"]["max_permissions"], json!(["auth.roles.read"]));
        assert_eq!(applied["data"]["guardrails"].as_array().unwrap().len(), 2);
        let (status, mine) = call(&app, "GET", "/guardrails", &mid, None).await;
        assert_eq!(status, StatusCode::OK, "{mine}");
        assert_eq!(mine["data"].as_array().unwrap().len(), 1);

        // Không rào được khách không ở dưới mình; permission lạ bị từ chối.
        let (status, problem) =
            call(&app, "PUT", &rail(&grandpa), &mid, Some(json!({"max_permissions": []}))).await;
        assert_eq!((status, problem["code"].as_str()), (StatusCode::FORBIDDEN, Some("auth.guardrail_not_a_child")));
        let (status, _) =
            call(&app, "PUT", &rail(&child), &old, Some(json!({"max_permissions": ["nope.read"]}))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

        // Gỡ rào của cha: chỉ còn rào của ông.
        let (status, _) = call(&app, "DELETE", &rail(&child), &mid, None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _) = call(&app, "DELETE", &rail(&child), &mid, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = call(&app, "GET", "/users", &young, None).await;
        assert_eq!(status, StatusCode::OK);
    })
    .await
    .unwrap();
}

// Tách khách đang bị rào (khách mới thành khách gốc, không còn cha): khách mới nhận bản sao mọi rào
// đang áp cho khách nguồn, nên không thoát rào; khách đặt rào vẫn sửa được rào đã chép.
#[tokio::test]
async fn tach_khach_dang_bi_rao_khach_moi_van_bi_rao() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let mailer: Arc<dyn Mailer> = services.mails.clone();
        let app = TestApp::builder(test)
            .tenant("parent")
            .tenant("child")
            .tenant("spinoff")
            .provide::<dyn Mailer>(mailer)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let parent = app.tenant("parent").await.unwrap();
        let child = app.tenant("child").await.unwrap();
        let spinoff = app.tenant("spinoff").await.unwrap();
        let control = app.database();
        control
            .write()
            .execute_raw(autoxide_kernel::database::statement(
                control.write().get_database_backend(),
                "UPDATE kernel_tenants SET parent_id = $1 WHERE id = $2",
                [parent.id().as_uuid().into(), child.id().as_uuid().into()],
            ))
            .await
            .unwrap();
        let boss = admin_of(&services, &app, &parent, "parent").await;
        let rail = |tenant: &TenantContext| format!("/guardrails/{}", tenant.id().as_uuid());
        let body = json!({"max_permissions": ["auth.roles.read"]});
        let (status, _) = call(&app, "PUT", &rail(&child), &boss, Some(body)).await;
        assert_eq!(status, StatusCode::OK);

        // Kernel ghi `kernel.relocation_started` khi tách `child` thành `spinoff` (khách gốc mới).
        let guardrails = crate::guardrail::GuardrailService::new(
            app.modules().tenancy().unwrap(),
            services.roles.catalog().clone(),
            autoxide_kernel::event::Outbox::standalone("auth"),
        );
        let (source, target) = (child.id().as_uuid(), spinoff.id().as_uuid());
        guardrails.carry_on_split(source, target).await.unwrap();
        guardrails.carry_on_split(source, target).await.unwrap();

        let young = admin_of(&services, &app, &spinoff, "spinoff").await;
        let (status, _) = call(&app, "GET", "/roles", &young, None).await;
        assert_eq!(status, StatusCode::OK);
        let (status, refused) = call(&app, "GET", "/users", &young, None).await;
        assert_eq!(
            (status, refused["reason"].as_str()),
            (StatusCode::FORBIDDEN, Some("outside_guardrail"))
        );
        let (status, applied) = call(&app, "GET", "/guardrails/applied", &young, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            applied["data"]["guardrails"].as_array().unwrap().len(),
            1,
            "chạy lại không nhân đôi"
        );

        // Khách đặt rào không còn ở trên `spinoff` nhưng vẫn gỡ được rào của mình.
        let (status, _) = call(&app, "DELETE", &rail(&spinoff), &boss, None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    })
    .await
    .unwrap();
}
