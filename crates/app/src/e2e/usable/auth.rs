//! `auth` dùng được như quản trị viên thật (đăng nhập thật, không người dùng giả), cả SQLite và
//! Postgres: role, lời mời qua thư, chỗ làm việc, đăng nhập bằng link, báo đổi email lạ, API key,
//! nhà cung cấp đăng nhập, SCIM, tài khoản khách và nhật ký audit.

use autoxide_kernel::tenancy::{DEFAULT_TENANT, root_of};
use autoxide_testkit::{TestApp, TestDatabase};
use serde_json::{Value, json};

use super::kit::{
    ADMIN_PASSWORD, ANONYMOUS, call_tool, drain, expect, id, mail_param, mails_to, real_app,
};

const SETTINGS: &[(&str, &str)] = &[
    ("AUTOXIDE_AUTH_INVITATION_URL", "https://app.example.com/invite?token={token}"),
    ("AUTOXIDE_AUTH_LOGIN_LINK", "true"),
    ("AUTOXIDE_AUTH_LOGIN_LINK_URL", "https://app.example.com/login?token={token}"),
    ("AUTOXIDE_AUTH_EMAIL_VERIFICATION_URL", "https://app.example.com/verify?token={token}"),
    ("AUTOXIDE_AUTH_EMAIL_CHANGE_REPORT_URL", "https://app.example.com/report?token={token}"),
    ("AUTOXIDE_AUTH_SELF_REGISTRATION", "true"),
];

/// Token mới nhất trong thư gửi tới `to` (sau khi chạy job gửi thư).
async fn token_mailed(app: &TestApp, mail_dir: &std::path::Path, to: &str) -> String {
    drain(app).await;
    let mails = mails_to(mail_dir, to);
    let last = mails.last().unwrap_or_else(|| panic!("no mail to {to}"));
    mail_param(last, "token")
}

async fn scim(app: &TestApp, method: &str, path: &str, token: &str, body: Value) -> Value {
    let method = axum::http::Method::from_bytes(method.as_bytes()).unwrap();
    let answer = app
        .request(method, &format!("/api/v1/auth/scim/v2{path}"))
        .token(token)
        .header("content-type", "application/scim+json")
        .bytes(body.to_string())
        .send()
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(answer.body()).unwrap_or(Value::Null);
    assert!(answer.status().is_success(), "{path}: {} {json}", answer.status());
    json
}

#[tokio::test]
async fn role_loi_moi_cho_lam_viec_va_dang_nhap_bang_link() {
    TestDatabase::run(async |test| {
        let real = real_app(test, SETTINGS).await;
        let (app, admin) = (&real.app, real.admin.as_str());
        let root = json!(root_of(DEFAULT_TENANT));

        // Role: tạo bằng tool, xem bằng REST và tool, thay, rồi một role khác để xoá.
        let role = call_tool(app, admin, "auth.create_role", json!({"name": "editor", "permissions": ["content.read"]})).await;
        let role_id = role["id"].as_str().unwrap_or_else(|| panic!("{role}")).to_owned();
        let got = expect(app, "GET", &format!("/api/v1/auth/roles/{role_id}"), admin, None, 200).await;
        assert_eq!(got["data"]["name"], "editor");
        call_tool(app, admin, "auth.get_role", json!({"id": role_id})).await;
        call_tool(
            app,
            admin,
            "auth.replace_role",
            json!({"id": role_id, "name": "editor", "permissions": ["content.read", "content.write_own"]}),
        )
        .await;
        let spare = call_tool(app, admin, "auth.create_role", json!({"name": "spare", "permissions": ["content.read"]})).await;
        call_tool(app, admin, "auth.delete_role", json!({"id": spare["id"]})).await;

        // Lời mời: tạo, liệt kê, gửi lại, nhận bằng token trong thư.
        let invited = expect(
            app,
            "POST",
            "/api/v1/auth/invitations",
            admin,
            Some(json!({"email": "lan@example.com", "name": "Lan", "role_ids": [role_id]})),
            201,
        )
        .await;
        let invitation = id(&invited);
        let listed = expect(app, "GET", "/api/v1/auth/invitations", admin, None, 200).await;
        assert!(listed["data"].as_array().unwrap().iter().any(|row| row["id"] == invitation.as_str()));
        call_tool(app, admin, "auth.list_invitations", json!({})).await;
        // Thư đầu đi bằng job; gửi lại thu hồi link cũ nên thư thứ hai mới dùng được.
        drain(app).await;
        expect(app, "POST", &format!("/api/v1/auth/invitations/{invitation}/resend"), admin, None, 200).await;
        let token = token_mailed(app, &real.mail_dir, "lan@example.com").await;
        let password = "lan's long enough password";
        let accepted = expect(
            app,
            "POST",
            "/api/v1/auth/invitations/accept",
            ANONYMOUS,
            Some(json!({"token": token, "name": "Lan", "password": password})),
            200,
        )
        .await;
        let lan = accepted["data"]["user_id"].as_str().unwrap_or_else(|| panic!("{accepted}")).to_owned();

        // Hai lời mời bị thu hồi: bằng REST và bằng tool.
        let other = call_tool(app, admin, "auth.create_invitation", json!({"email": "an@example.com", "role_ids": [role_id]})).await;
        expect(app, "DELETE", &format!("/api/v1/auth/invitations/{}", other["id"].as_str().unwrap()), admin, None, 200).await;
        let third = call_tool(app, admin, "auth.create_invitation", json!({"email": "binh@example.com", "role_ids": [role_id]})).await;
        call_tool(app, admin, "auth.revoke_invitation", json!({"id": third["id"]})).await;

        // Chỗ làm việc và role của Lan.
        let memberships = format!("/api/v1/auth/users/{lan}/memberships");
        let body = json!({"memberships": [{"org_id": root, "role_ids": [role_id]}]});
        expect(app, "PUT", &memberships, admin, Some(body.clone()), 200).await;
        let current = expect(app, "GET", &memberships, admin, None, 200).await;
        assert_eq!(current["data"].as_array().unwrap().len(), 1, "{current}");
        let mut tool_body = body;
        tool_body["user_id"] = json!(lan);
        call_tool(app, admin, "auth.set_memberships", tool_body).await;
        call_tool(app, admin, "auth.set_user_roles", json!({"user_id": lan, "role_ids": [role_id]})).await;
        call_tool(app, admin, "auth.get_user_roles", json!({"id": lan})).await;

        // Đăng nhập bằng link gửi qua thư.
        expect(app, "POST", "/api/v1/auth/login/link", ANONYMOUS, Some(json!({"email": "lan@example.com"})), 202).await;
        let token = token_mailed(app, &real.mail_dir, "lan@example.com").await;
        let signed_in =
            expect(app, "POST", "/api/v1/auth/login/link/confirm", ANONYMOUS, Some(json!({"token": token})), 200).await;
        let lan_token = signed_in["data"]["access_token"].as_str().unwrap().to_owned();

        // Lan đổi email; địa chỉ cũ nhận thư "không phải tôi" và báo lại.
        let change = json!({"current_password": password, "new_email": "lan.new@example.com"});
        expect(app, "POST", "/api/v1/auth/me/email", &lan_token, Some(change), 202).await;
        let token = token_mailed(app, &real.mail_dir, "lan.new@example.com").await;
        expect(app, "POST", "/api/v1/auth/email-verification/confirm", &lan_token, Some(json!({"token": token})), 200).await;
        let token = token_mailed(app, &real.mail_dir, "lan@example.com").await;
        expect(app, "POST", "/api/v1/auth/email-change/report", ANONYMOUS, Some(json!({"token": token})), 204).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn api_key_nha_cung_cap_dang_nhap_scim_va_khach() {
    TestDatabase::run(async |test| {
        let real = real_app(test, SETTINGS).await;
        let (app, admin) = (&real.app, real.admin.as_str());
        let root = json!(root_of(DEFAULT_TENANT));

        // API key của chính mình: tạo, xoay, thu hồi bằng tool; một key khác bị quản trị viên thu hồi.
        let key = call_tool(
            app,
            admin,
            "auth.create_api_key",
            json!({"name": "agent", "permissions": ["content.read"], "current_password": ADMIN_PASSWORD}),
        )
        .await;
        let key_id = key["id"].as_str().unwrap_or_else(|| panic!("{key}")).to_owned();
        let rotated = call_tool(
            app,
            admin,
            "auth.rotate_api_key",
            json!({"id": key_id, "current_password": ADMIN_PASSWORD}),
        )
        .await;
        let rotated_id = rotated["id"].as_str().unwrap_or_else(|| panic!("{rotated}")).to_owned();
        call_tool(app, admin, "auth.revoke_api_key", json!({"id": rotated_id})).await;
        let body = json!({"name": "ci", "permissions": ["content.read"], "current_password": ADMIN_PASSWORD});
        let created = expect(app, "POST", "/api/v1/auth/api-keys", admin, Some(body), 201).await;
        expect(app, "DELETE", &format!("/api/v1/auth/all-api-keys/{}", id(&created)), admin, None, 204).await;

        // Nhà cung cấp đăng nhập (OIDC).
        let provider = call_tool(
            app,
            admin,
            "auth.create_oidc_provider",
            json!({"kind": "google", "name": "Google", "slug": "google", "client_id": "client-1",
                   "client_secret": "secret-1", "redirect_uris": ["https://app.example.com/callback"]}),
        )
        .await;
        let provider_id = provider["id"].as_str().unwrap_or_else(|| panic!("{provider}")).to_owned();
        let listed = expect(app, "GET", "/api/v1/auth/oidc-providers", admin, None, 200).await;
        assert_eq!(listed["data"].as_array().unwrap().len(), 1, "{listed}");
        expect(app, "GET", &format!("/api/v1/auth/oidc-providers/{provider_id}"), admin, None, 200).await;
        call_tool(app, admin, "auth.list_oidc_providers", json!({})).await;
        call_tool(app, admin, "auth.update_oidc_provider", json!({"id": provider_id, "name": "Google Workspace"})).await;
        call_tool(app, admin, "auth.delete_oidc_provider", json!({"id": provider_id})).await;

        // SCIM: token, người dùng, nhóm, quy tắc nhóm → role.
        let created = expect(app, "POST", "/api/v1/auth/scim/tokens", admin, Some(json!({"name": "Entra"})), 201).await;
        let secret = created["data"]["token_secret"].as_str().unwrap().to_owned();
        let user = scim(
            app,
            "POST",
            "/Users",
            &secret,
            json!({"schemas": ["urn:ietf:params:scim:schemas:core:2.0:User"], "userName": "grace@example.com",
                   "emails": [{"value": "grace@example.com", "primary": true}], "active": true}),
        )
        .await;
        let user_id = user["id"].as_str().unwrap().to_owned();
        scim(
            app,
            "PUT",
            &format!("/Users/{user_id}"),
            &secret,
            json!({"schemas": ["urn:ietf:params:scim:schemas:core:2.0:User"], "userName": "grace@example.com",
                   "displayName": "Grace Hopper", "emails": [{"value": "grace@example.com", "primary": true}],
                   "active": true}),
        )
        .await;
        let group = scim(
            app,
            "POST",
            "/Groups",
            &secret,
            json!({"schemas": ["urn:ietf:params:scim:schemas:core:2.0:Group"], "displayName": "Editors"}),
        )
        .await;
        let group_id = group["id"].as_str().unwrap().to_owned();
        scim(
            app,
            "PUT",
            &format!("/Groups/{group_id}"),
            &secret,
            json!({"schemas": ["urn:ietf:params:scim:schemas:core:2.0:Group"], "displayName": "Editors",
                   "members": [{"value": user_id}]}),
        )
        .await;
        let role = call_tool(app, admin, "auth.create_role", json!({"name": "editor", "permissions": ["content.read"]})).await;
        let mapping = json!({"group_id": group_id, "org_id": root, "role_id": role["id"]});
        expect(app, "POST", "/api/v1/auth/scim/group-mappings", admin, Some(mapping), 201).await;
        let mappings = expect(app, "GET", "/api/v1/auth/scim/group-mappings", admin, None, 200).await;
        assert_eq!(mappings["data"].as_array().unwrap().len(), 1, "{mappings}");

        // Khách tự đăng ký; nhân viên xem tài khoản khách.
        let body = json!({"email": "buyer@example.com", "name": "Buyer", "password": "a long buyer password", "realm": "customer"});
        expect(app, "POST", "/api/v1/auth/registrations", ANONYMOUS, Some(body), 202).await;
        let customers = expect(app, "GET", "/api/v1/auth/customers", admin, None, 200).await;
        let customer = customers["data"][0]["id"].as_str().unwrap_or_else(|| panic!("{customers}")).to_owned();
        expect(app, "GET", &format!("/api/v1/auth/customers/{customer}"), admin, None, 200).await;

        // Nhật ký audit có các việc trên.
        drain(app).await;
        let entries = call_tool(app, admin, "audit.list_entries", json!({})).await;
        let entry = entries[0]["id"].clone();
        assert!(entry.is_string(), "{entries}");
        call_tool(app, admin, "audit.get_entry", json!({"id": entry})).await;
    })
    .await
    .unwrap();
}
