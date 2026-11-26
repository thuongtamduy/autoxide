//! Luồng OAuth đầu-cuối qua REST và MCP (D-20261126-mcp-oauth), cả SQLite và Postgres: đăng ký client, `/authorize`,
//! consent của người dùng, đổi mã lấy token, gọi MCP, xoay refresh token, mã dùng lại thu hồi grant.

use autoxide_kernel::event::Actor;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::{StatusCode, header};
use serde_json::{Value, json};

use crate::http::test_support::{PASSWORD, app_built, bearer, call};
use crate::user::NewUser;

const PUBLIC: &str = "https://api.example.test";
const CONSENT: &str = "https://app.example.test/connect?request={token}";
const REDIRECT: &str = "http://127.0.0.1/cb";
/// RFC 7636 phụ lục B.
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

async fn oauth_app(test: &TestDatabase) -> (TestApp, String) {
    let (app, services) = app_built(test, false, |builder| {
        builder
            .setting("AUTOXIDE_HTTP_PUBLIC_URL", PUBLIC)
            .setting("AUTOXIDE_AUTH_OAUTH_CONSENT_URL", CONSENT)
    })
    .await;
    let boss = NewUser {
        email: "boss@example.com".to_owned(),
        name: "Boss".to_owned(),
        password: Some(PASSWORD.to_owned()),
    };
    services.users.create_with_roles(&services.tenant, Actor::system(), boss, true).await.unwrap();
    let session = bearer(&app, "boss@example.com", PASSWORD).await;
    (app, session)
}

/// Query của một URL thành bảng tên → giá trị.
fn query_of(url: &str) -> std::collections::HashMap<String, String> {
    reqwest::Url::parse(url).unwrap().query_pairs().into_owned().collect()
}

fn encode(value: &str) -> String {
    reqwest::Url::parse_with_params("http://x/", [("v", value)])
        .unwrap()
        .query()
        .unwrap()
        .trim_start_matches("v=")
        .to_owned()
}

async fn form(app: &TestApp, path: &str, pairs: &[(&str, &str)]) -> (StatusCode, Value) {
    let body = pairs
        .iter()
        .map(|(name, value)| format!("{name}={}", encode(value)))
        .collect::<Vec<_>>()
        .join("&");
    let answer = app
        .post(&format!("/api/v1/auth{path}"))
        .header(header::CONTENT_TYPE.as_str(), "application/x-www-form-urlencoded")
        .bytes(body)
        .unchecked()
        .send()
        .await
        .unwrap();
    let json = serde_json::from_slice(answer.body()).unwrap_or(Value::Null);
    (answer.status(), json)
}

/// Đăng ký động, `/authorize`, người dùng duyệt `permissions`: trả `client_id` và mã.
async fn approved_code(app: &TestApp, session: &str, permissions: &[&str]) -> (String, String) {
    let registration = json!({"redirect_uris": [REDIRECT], "client_name": "Test client"});
    let (status, _, client) = call(app, "POST", "/oauth/register", None, Some(registration)).await;
    assert_eq!(status, StatusCode::CREATED, "{client}");
    let client_id = client["client_id"].as_str().unwrap().to_owned();
    let authorize = format!(
        "/api/v1/auth/oauth/authorize?response_type=code&client_id={client_id}&redirect_uri={}&code_challenge={CHALLENGE}\
         &code_challenge_method=S256&state=xyz&resource={}",
        // Cổng loopback do client chọn lúc chạy: khớp bỏ qua cổng.
        encode("http://127.0.0.1:51234/cb"),
        encode(&format!("{PUBLIC}/mcp")),
    );
    let answer = app.get(&authorize).send().await.unwrap();
    assert_eq!(answer.status(), StatusCode::FOUND);
    let location = answer.header(header::LOCATION).to_owned();
    assert!(location.starts_with("https://app.example.test/connect?request="), "{location}");
    let token = query_of(&location)["request"].clone();

    let (status, _, request) =
        call(app, "GET", &format!("/oauth/requests/{token}"), Some(session), None).await;
    assert_eq!(status, StatusCode::OK, "{request}");
    assert_eq!(request["data"]["client_name"], "Test client");
    assert_eq!(request["data"]["redirect_host"], "127.0.0.1");
    assert_eq!(request["data"]["localhost_redirect"], true);
    assert_eq!(request["data"]["resource"], format!("{PUBLIC}/mcp"));
    let decision = json!({"permissions": permissions});
    let (status, _, approved) = call(
        app,
        "POST",
        &format!("/oauth/requests/{token}/approve"),
        Some(session),
        Some(decision),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{approved}");
    let redirect = approved["data"]["redirect_to"].as_str().unwrap();
    assert!(redirect.starts_with("http://127.0.0.1:51234/cb?"), "{redirect}");
    let answer = query_of(redirect);
    assert_eq!(answer["state"], "xyz");
    assert_eq!(answer["iss"], PUBLIC);
    // Yêu cầu chỉ dùng một lần.
    let (status, _, _) =
        call(app, "POST", &format!("/oauth/requests/{token}/deny"), Some(session), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    (client_id, answer["code"].clone())
}

async fn exchange(
    app: &TestApp,
    client_id: &str,
    code: &str,
    verifier: &str,
) -> (StatusCode, Value) {
    form(
        app,
        "/oauth/token",
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("code_verifier", verifier),
            ("client_id", client_id),
            ("redirect_uri", "http://127.0.0.1:51234/cb"),
            ("resource", &format!("{PUBLIC}/mcp")),
        ],
    )
    .await
}

async fn mcp_status(app: &TestApp, token: &str) -> StatusCode {
    app.post("/mcp")
        .header(header::AUTHORIZATION.as_str(), &format!("Bearer {token}"))
        .header(header::ACCEPT.as_str(), "application/json, text/event-stream")
        .json(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))
        .unchecked()
        .send()
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn luong_day_du_tu_dang_ky_toi_goi_mcp_va_thu_hoi() {
    TestDatabase::run(async |test| {
        let (app, session) = oauth_app(test).await;

        let metadata =
            app.get("/.well-known/oauth-authorization-server").send().await.unwrap().json();
        assert_eq!(metadata["issuer"], PUBLIC);
        assert_eq!(metadata["code_challenge_methods_supported"], json!(["S256"]));
        assert_eq!(metadata["client_id_metadata_document_supported"], true);
        assert_eq!(metadata["token_endpoint"], format!("{PUBLIC}/api/v1/auth/oauth/token"));

        let (client_id, code) = approved_code(&app, &session, &["notes.write"]).await;
        // PKCE sai: từ chối, và mã đã bị đánh dấu dùng nên không thử lại được.
        let wrong = "x".repeat(43);
        let (status, error) = exchange(&app, &client_id, &code, &wrong).await;
        assert_eq!(
            (status, error["error"].as_str()),
            (StatusCode::BAD_REQUEST, Some("invalid_grant"))
        );

        let (client_id, code) = approved_code(&app, &session, &["notes.write"]).await;
        let (status, tokens) = exchange(&app, &client_id, &code, VERIFIER).await;
        assert_eq!(status, StatusCode::OK, "{tokens}");
        assert_eq!(tokens["token_type"], "Bearer");
        assert_eq!(tokens["scope"], "notes.write");
        let access = tokens["access_token"].as_str().unwrap().to_owned();
        assert!(access.starts_with("axo1_"));
        assert_eq!(mcp_status(&app, &access).await, StatusCode::OK);
        // Token cho `/mcp` không dùng được ở REST.
        let (status, _, refused) =
            call(&app, "GET", "/me", Some(&format!("Bearer {access}")), None).await;
        assert_eq!(
            (status, refused["code"].as_str()),
            (StatusCode::UNAUTHORIZED, Some("oauth_token_audience"))
        );

        // Xoay refresh token; gửi lại token liền trước ngay sau đó: chỉ bị từ chối, grant còn sống.
        let refresh = tokens["refresh_token"].as_str().unwrap().to_owned();
        let refreshing = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh.as_str()),
            ("client_id", client_id.as_str()),
        ];
        let (status, renewed) = form(&app, "/oauth/token", &refreshing).await;
        assert_eq!(status, StatusCode::OK, "{renewed}");
        assert_ne!(renewed["refresh_token"], tokens["refresh_token"]);
        let (status, _) = form(&app, "/oauth/token", &refreshing).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let renewed_access = renewed["access_token"].as_str().unwrap().to_owned();
        assert_eq!(mcp_status(&app, &renewed_access).await, StatusCode::OK);

        // Mã dùng lại: thu hồi cả grant, mọi token của nó ngừng ngay.
        let (status, _) = exchange(&app, &client_id, &code, VERIFIER).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(mcp_status(&app, &renewed_access).await, StatusCode::UNAUTHORIZED);

        let (status, _, grants) = call(&app, "GET", "/me/oauth-grants", Some(&session), None).await;
        assert_eq!(status, StatusCode::OK, "{grants}");
        let list = grants["data"].as_array().unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.iter().all(|grant| grant["client_host"].is_string()));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nguoi_dung_thu_hoi_grant_va_redirect_la_khong_chuyen_huong() {
    TestDatabase::run(async |test| {
        let (app, session) = oauth_app(test).await;
        let (client_id, code) = approved_code(&app, &session, &["notes.write"]).await;
        let (_, tokens) = exchange(&app, &client_id, &code, VERIFIER).await;
        let access = tokens["access_token"].as_str().unwrap().to_owned();

        // Token OAuth không tự quản lý grant (chỉ phiên đăng nhập).
        let (_, _, grants) = call(&app, "GET", "/me/oauth-grants", Some(&session), None).await;
        let id = grants["data"][0]["id"].as_str().unwrap().to_owned();
        let (status, _, revoked) =
            call(&app, "DELETE", &format!("/me/oauth-grants/{id}"), Some(&session), None).await;
        assert_eq!(status, StatusCode::OK, "{revoked}");
        assert!(revoked["data"]["revoked_at"].is_string());
        assert_eq!(mcp_status(&app, &access).await, StatusCode::UNAUTHORIZED);

        // `redirect_uri` client không đăng ký: 400 ở đây, không bao giờ chuyển hướng tới đó.
        let authorize = format!(
            "/api/v1/auth/oauth/authorize?response_type=code&client_id={client_id}&redirect_uri={}\
             &code_challenge={CHALLENGE}&code_challenge_method=S256",
            encode("https://evil.example/cb"),
        );
        let answer = app.get(&authorize).unchecked().send().await.unwrap();
        assert_eq!(answer.status(), StatusCode::BAD_REQUEST);
        // Thiếu PKCE: chuyển hướng về client với lỗi, kèm `iss`.
        let authorize = format!(
            "/api/v1/auth/oauth/authorize?response_type=code&client_id={client_id}&redirect_uri={}&state=s",
            encode(REDIRECT),
        );
        let answer = app.get(&authorize).send().await.unwrap();
        assert_eq!(answer.status(), StatusCode::FOUND);
        let query = query_of(answer.header(header::LOCATION));
        assert_eq!(query["error"], "invalid_request");
        assert_eq!(query["iss"], PUBLIC);
        assert_eq!(query["state"], "s");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hai_lan_doi_cung_refresh_token_dong_thoi_chi_mot_lan_thang() {
    TestDatabase::run(async |test| {
        let (app, session) = oauth_app(test).await;
        let (client_id, code) = approved_code(&app, &session, &["notes.write"]).await;
        let (_, tokens) = exchange(&app, &client_id, &code, VERIFIER).await;
        let refresh = tokens["refresh_token"].as_str().unwrap().to_owned();
        let pairs = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh.as_str()),
            ("client_id", client_id.as_str()),
        ];
        let (first, second) =
            tokio::join!(form(&app, "/oauth/token", &pairs), form(&app, "/oauth/token", &pairs));
        let wins = [first.0, second.0].iter().filter(|status| **status == StatusCode::OK).count();
        assert_eq!(wins, 1, "{first:?} {second:?}");
    })
    .await
    .unwrap();
}
