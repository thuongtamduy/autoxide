//! Xác nhận lại cho việc nhạy cảm (bước 54 P5, D-20261119-step54-step-up), qua REST với module `auth`
//! thật, trên cả SQLite và Postgres: phiên cũ bị đòi xác nhận lại cho permission nhạy cảm, xác nhận
//! xong thì làm được (kể cả sau refresh), khách tắt thì không đòi, API key không bị đòi.

use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};

use crate::api::{Accounts, NewAccount};
use crate::http::test_support::{PASSWORD, app, call};
use crate::session::entity::{Column, Entity};

async fn login(app: &autoxide_testkit::TestApp, email: &str) -> Value {
    let body = json!({"email": email, "password": PASSWORD});
    let (status, _, answer) = call(app, "POST", "/login", None, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    answer["data"].clone()
}

#[tokio::test]
async fn viec_nhay_cam_doi_xac_thuc_lai() {
    TestDatabase::run(async |test| {
        let (app, services) = app(test).await;
        let tenant = services.tenant.clone();
        let root = NewAccount {
            email: "root@example.com".to_owned(),
            name: "Root".to_owned(),
            password: Some(PASSWORD.to_owned()),
            admin: true,
        };
        Accounts::create(services.users.as_ref(), &tenant, root).await.unwrap();
        let tokens = login(&app, "root@example.com").await;
        let fresh = format!("Bearer {}", tokens["access_token"].as_str().unwrap());
        // Vừa đăng nhập: việc nhạy cảm (`auth.users.read`, dữ liệu cá nhân) làm được.
        let (status, _, _) = call(&app, "GET", "/users", Some(&fresh), None).await;
        assert_eq!(status, StatusCode::OK);

        // Phiên đã 20 phút: refresh cấp token mang `auth_time` cũ.
        let old = chrono::Utc::now() - chrono::Duration::minutes(20);
        Entity::update_many()
            .col_expr(Column::AuthenticatedAt, Expr::value(Some(old)))
            .filter(Column::TenantId.eq(tenant.id().as_uuid()))
            .exec(tenant.database().write())
            .await
            .unwrap();
        let body = json!({"refresh_token": tokens["refresh_token"]});
        let (status, _, renewed) = call(&app, "POST", "/refresh", None, Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{renewed}");
        let stale = format!("Bearer {}", renewed["data"]["access_token"].as_str().unwrap());
        let (status, headers, refused) = call(&app, "GET", "/users", Some(&stale), None).await;
        assert_eq!(
            (status, refused["code"].as_str()),
            (StatusCode::UNAUTHORIZED, Some("step_up_required"))
        );
        let challenge = headers["www-authenticate"].to_str().unwrap();
        assert!(challenge.contains("insufficient_user_authentication"), "{challenge}");
        assert!(challenge.contains("max_age=900"), "{challenge}");
        // Việc thường vẫn làm được.
        let (status, _, _) = call(&app, "GET", "/roles", Some(&stale), None).await;
        assert_eq!(status, StatusCode::OK);

        // Xác nhận lại: nhận access token mới, việc nhạy cảm làm được ngay.
        let proof = json!({"method": "password", "password": PASSWORD});
        let (status, _, confirmed) =
            call(&app, "POST", "/me/reauth", Some(&stale), Some(proof)).await;
        assert_eq!(status, StatusCode::OK, "{confirmed}");
        let again = format!("Bearer {}", confirmed["data"]["access_token"].as_str().unwrap());
        let (status, _, _) = call(&app, "GET", "/users", Some(&again), None).await;
        assert_eq!(status, StatusCode::OK);
        // Refresh sau đó giữ lần xác nhận.
        let body = json!({"refresh_token": renewed["data"]["refresh_token"]});
        let (status, _, kept) = call(&app, "POST", "/refresh", None, Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{kept}");
        let kept = format!("Bearer {}", kept["data"]["access_token"].as_str().unwrap());
        let (status, _, _) = call(&app, "GET", "/users", Some(&kept), None).await;
        assert_eq!(status, StatusCode::OK);

        // Khách tắt `auth.step_up`: token cũ cũng làm được.
        let answer = app
            .request(axum::http::Method::PUT, "/api/v1/kernel/settings/auth.step_up")
            .header("authorization", &again)
            .json(json!({"value": "false"}))
            .send()
            .await
            .unwrap();
        assert!(answer.status().is_success(), "{answer:?}");
        let (status, _, _) = call(&app, "GET", "/users", Some(&stale), None).await;
        assert_eq!(status, StatusCode::OK);
    })
    .await
    .unwrap();
}
