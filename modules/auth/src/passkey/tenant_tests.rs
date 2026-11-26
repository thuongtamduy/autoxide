//! Passkey theo khách (D-20261126-tenant-passkey-rp): RP ID là tên miền ĐANG xác minh của chính khách
//! (`auth.passkey_rp_id`), kiểm lúc đặt và lúc dùng. Tên miền chuyển sang khách khác thì passkey đăng ký trên đó
//! không đăng nhập vào khách cũ được nữa (trang của khách mới không mượn được passkey của người dùng khách cũ).
//! Chạy trên cả SQLite và Postgres.

use autoxide_kernel::event::Actor;
use autoxide_kernel::tenancy::TenantDomains;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use serde_json::{Value, json};

use super::testing::SoftAuthenticator;
use crate::http::test_support::{PASSWORD, app_built, call_with};
use crate::user::NewUser;

const HOME: [(&str, &str); 1] = [("autoxide-tenant", "default")];

async fn signed_in(app: &TestApp, email: &str) -> String {
    let body = json!({"email": email, "password": PASSWORD});
    let (status, _, login) = call_with(app, "POST", "/login", &HOME, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{login}");
    format!("Bearer {}", login["data"]["access_token"].as_str().unwrap())
}

async fn put_rp(app: &TestApp, admin: &str, value: &str) -> (StatusCode, Value) {
    let response = app
        .put("/api/v1/kernel/settings/auth.passkey_rp_id")
        .header("authorization", admin)
        .json(json!({"value": value}))
        .send()
        .await
        .unwrap();
    (response.status(), response.json())
}

async fn sign_in_with(app: &TestApp, device: &mut SoftAuthenticator) -> (StatusCode, Value) {
    let (status, _, options) = call_with(app, "POST", "/login/passkey/options", &HOME, None).await;
    assert_eq!(status, StatusCode::OK, "{options}");
    let ceremony = options["data"]["ceremony_id"].clone();
    let credential = device.sign_in(&options["data"]["options"]);
    let body = json!({"ceremony_id": ceremony, "credential": credential});
    let (status, _, answer) = call_with(app, "POST", "/login/passkey", &HOME, Some(body)).await;
    (status, answer)
}

#[tokio::test]
async fn rp_id_la_ten_mien_dang_xac_minh_cua_khach() {
    TestDatabase::run(async |test| {
        let (app, services) =
            app_built(test, false, |builder| builder.domain("default", "shop-a.test").tenant("c"))
                .await;
        let root = NewUser {
            email: "root@example.com".to_owned(),
            name: "Root".to_owned(),
            password: Some(PASSWORD.to_owned()),
        };
        services.users.create_with_roles(&services.tenant, Actor::system(), root, true).await.unwrap();
        let admin = signed_in(&app, "root@example.com").await;

        // Lúc đặt: chỉ tên miền đã xác minh của chính khách, đúng dạng.
        for (value, code) in [
            ("evil.test", "kernel.domain_not_verified"),
            ("https://shop-a.test", "kernel.domain_name_invalid"),
            ("Shop-A.test", "kernel.domain_name_invalid"),
            ("1.2.3.4", "kernel.domain_name_invalid"),
        ] {
            let (status, body) = put_rp(&app, &admin, value).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{value}: {body}");
            assert_eq!(body["errors"][0]["code"], code, "{value}: {body}");
        }
        let (status, body) = put_rp(&app, &admin, "shop-a.test").await;
        assert_eq!(status, StatusCode::OK, "{body}");

        // Đăng ký passkey trên `shop-a.test` rồi đăng nhập bằng nó.
        let ada = signed_in(&app, "ada@example.com").await;
        let headers = [("authorization", ada.as_str())];
        let body = json!({"current_password": PASSWORD});
        let (status, _, started) =
            call_with(&app, "POST", "/me/passkeys/registration", &headers, Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{started}");
        let options = &started["data"]["options"];
        let rp = options["publicKey"]["rp"]["id"].as_str().or(options["rp"]["id"].as_str());
        assert_eq!(rp, Some("shop-a.test"), "{options}");
        let mut device = SoftAuthenticator::new(7).on("shop-a.test");
        let credential = device.register(options);
        let body = json!({"ceremony_id": started["data"]["ceremony_id"], "name": "Phone", "credential": credential});
        let (status, _, added) = call_with(&app, "POST", "/me/passkeys", &headers, Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{added}");
        let (status, answer) = sign_in_with(&app, &mut device).await;
        assert_eq!(status, StatusCode::OK, "{answer}");
        let config = app
            .get("/api/v1/kernel/client-config")
            .header("autoxide-tenant", "default")
            .send()
            .await
            .unwrap()
            .json();
        assert_eq!(config["data"]["auth"]["passkey_rp_id"], "shop-a.test", "{config}");

        // Tên miền chuyển sang khách `c`: khách cũ không còn nhận passkey trên `shop-a.test`.
        let domains = app.api::<TenantDomains>().unwrap();
        let home = services.tenant.clone();
        let c = app.tenant("c").await.unwrap();
        let page = autoxide_kernel::pagination::PageRequest::default();
        let listed = domains.list(&home, &page).await.unwrap().data;
        domains.revoke(Actor::system(), &home, listed[0].id).await.unwrap();
        let moved = domains.add(Actor::system(), &c, "shop-a.test").await.unwrap();
        app.dns().set_txt(&moved.record.name, &[&moved.record.value]);
        assert_eq!(domains.verify(Actor::system(), &c, moved.id).await.unwrap().status, "verified");

        let (status, answer) = sign_in_with(&app, &mut device).await;
        assert_ne!(status, StatusCode::OK, "{answer}");
        // Giá trị cũ vẫn lưu nhưng được báo là không còn dùng.
        let response = app
            .get("/api/v1/kernel/settings")
            .header("authorization", &admin)
            .send()
            .await
            .unwrap();
        let listed = response.json();
        let rp = listed["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|setting| setting["name"] == "auth.passkey_rp_id")
            .cloned()
            .unwrap();
        assert_eq!(rp["problem"], "kernel.domain_unverified", "{rp}");
    })
    .await
    .unwrap();
}
