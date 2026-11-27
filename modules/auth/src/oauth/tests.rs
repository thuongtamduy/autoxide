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
/// Cặp PKCE sinh ngẫu nhiên mỗi lần chạy: test không viết cứng giá trị bí mật nào. `LazyLock`: tạo một lần khi
/// dùng lần đầu, dùng chung cho mọi test của chương trình.
static PKCE: std::sync::LazyLock<(String, String)> = std::sync::LazyLock::new(|| {
    use base64::Engine as _;
    use sha2::Digest as _;
    let verifier = autoxide_kernel::secret::random_token(32).unwrap();
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(sha2::Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
});

/// `code_verifier` của các test (RFC 7636).
fn pkce_verifier() -> &'static str {
    &PKCE.0
}

/// `code_challenge` (S256) khớp với [`pkce_verifier`].
fn pkce_challenge() -> &'static str {
    &PKCE.1
}

async fn oauth_app(test: &TestDatabase) -> (TestApp, String) {
    let (app, services) = app_built(test, false, |builder| {
        builder
            .setting("AUTOXIDE_HTTP_PUBLIC_URL", PUBLIC)
            .setting("AUTOXIDE_AUTH_OAUTH_CONSENT_URL", CONSENT)
            // Máy chủ tài liệu metadata của client trong test chạy ở 127.0.0.1.
            .setting("AUTOXIDE_AUTH_OAUTH_ALLOW_PRIVATE_NETWORKS", "true")
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
    approved_code_for(app, session, permissions, &format!("{PUBLIC}/mcp")).await
}

/// Như [`approved_code`], cho resource `resource`.
async fn approved_code_for(
    app: &TestApp,
    session: &str,
    permissions: &[&str],
    resource: &str,
) -> (String, String) {
    let registration = json!({"redirect_uris": [REDIRECT], "client_name": "Test client"});
    let (status, _, client) = call(app, "POST", "/oauth/register", None, Some(registration)).await;
    assert_eq!(status, StatusCode::CREATED, "{client}");
    let client_id = client["client_id"].as_str().unwrap().to_owned();
    let challenge = pkce_challenge();
    let authorize = format!(
        "/api/v1/auth/oauth/authorize?response_type=code&client_id={client_id}&redirect_uri={}&code_challenge={challenge}\
         &code_challenge_method=S256&state=xyz&resource={}",
        // Cổng loopback do client chọn lúc chạy: khớp bỏ qua cổng.
        encode("http://127.0.0.1:51234/cb"),
        encode(resource),
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
    assert_eq!(request["data"]["resource"], resource);
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
    exchange_for(app, client_id, code, verifier, &format!("{PUBLIC}/mcp")).await
}

async fn exchange_for(
    app: &TestApp,
    client_id: &str,
    code: &str,
    verifier: &str,
    resource: &str,
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
            ("resource", resource),
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
        let (status, tokens) = exchange(&app, &client_id, &code, pkce_verifier()).await;
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

        // Xoay refresh token; gửi lại token đã dùng: thu hồi cả grant, không ân hạn (RFC 9700 §4.14.2).
        let refresh = tokens["refresh_token"].as_str().unwrap().to_owned();
        let refreshing = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh.as_str()),
            ("client_id", client_id.as_str()),
        ];
        let (status, renewed) = form(&app, "/oauth/token", &refreshing).await;
        assert_eq!(status, StatusCode::OK, "{renewed}");
        assert_ne!(renewed["refresh_token"], tokens["refresh_token"]);
        let renewed_access = renewed["access_token"].as_str().unwrap().to_owned();
        assert_eq!(mcp_status(&app, &renewed_access).await, StatusCode::OK);
        let (status, _) = form(&app, "/oauth/token", &refreshing).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(mcp_status(&app, &renewed_access).await, StatusCode::UNAUTHORIZED);
        let next = [
            ("grant_type", "refresh_token"),
            ("refresh_token", renewed["refresh_token"].as_str().unwrap()),
            ("client_id", client_id.as_str()),
        ];
        assert_eq!(form(&app, "/oauth/token", &next).await.0, StatusCode::BAD_REQUEST);

        // Mã dùng lại: thu hồi cả grant, mọi token của nó ngừng ngay.
        let (client_id, code) = approved_code(&app, &session, &["notes.write"]).await;
        let (_, tokens) = exchange(&app, &client_id, &code, pkce_verifier()).await;
        let access = tokens["access_token"].as_str().unwrap().to_owned();
        assert_eq!(mcp_status(&app, &access).await, StatusCode::OK);
        let (status, _) = exchange(&app, &client_id, &code, pkce_verifier()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(mcp_status(&app, &access).await, StatusCode::UNAUTHORIZED);

        let (status, _, grants) = call(&app, "GET", "/me/oauth-grants", Some(&session), None).await;
        assert_eq!(status, StatusCode::OK, "{grants}");
        let list = grants["data"].as_array().unwrap();
        assert_eq!(list.len(), 3);
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
        let (_, tokens) = exchange(&app, &client_id, &code, pkce_verifier()).await;
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
        let challenge = pkce_challenge();
        let authorize = format!(
            "/api/v1/auth/oauth/authorize?response_type=code&client_id={client_id}&redirect_uri={}\
             &code_challenge={challenge}&code_challenge_method=S256",
            encode("https://evil.example/cb"),
        );
        let answer = app.get(&authorize).unchecked().send().await.unwrap();
        assert_eq!(answer.status(), StatusCode::BAD_REQUEST);
        // Thiếu PKCE: 400 tại chỗ, không chuyển hướng lỗi về client tự đăng ký (RFC 9700 §4.11.2).
        let authorize = format!(
            "/api/v1/auth/oauth/authorize?response_type=code&client_id={client_id}&redirect_uri={}&state=s",
            encode(REDIRECT),
        );
        let answer = app.get(&authorize).send().await.unwrap();
        assert_eq!(answer.status(), StatusCode::BAD_REQUEST);
        assert_eq!(answer.json()["code"], "auth.oauth_request_invalid");
        // `client_id` quá dài: 400.
        let long = format!("https://client.example/{}", "a".repeat(2100));
        let authorize = format!("/api/v1/auth/oauth/authorize?client_id={}", encode(&long));
        assert_eq!(app.get(&authorize).send().await.unwrap().status(), StatusCode::BAD_REQUEST);
    })
    .await
    .unwrap();
}

/// `POST /oauth/register` với thân `body`: mã HTTP và thân trả về.
async fn register(app: &TestApp, body: Value) -> (StatusCode, Value) {
    let (status, _, answer) = call(app, "POST", "/oauth/register", None, Some(body)).await;
    (status, answer)
}

/// Đầu vào quá cỡ của các endpoint nhận body thô bị chặn trước mọi xử lý, lỗi đúng định dạng RFC.
#[tokio::test]
async fn dang_ky_va_form_qua_co_bi_tu_choi_theo_rfc() {
    TestDatabase::run(async |test| {
        let (app, _) = oauth_app(test).await;
        // Quá 10 redirect_uri: lỗi của redirect.
        let many: Vec<String> = (0..11).map(|port| format!("http://127.0.0.1:{port}/cb")).collect();
        let (status, answer) = register(&app, json!({"redirect_uris": many})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(answer["error"], "invalid_redirect_uri");
        // Tên quá dài, danh sách grant quá dài: lỗi metadata, nêu tên trường.
        let (status, answer) =
            register(&app, json!({"redirect_uris": [REDIRECT], "client_name": "n".repeat(1001)}))
                .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(answer["error"], "invalid_client_metadata");
        assert!(answer["error_description"].as_str().unwrap().contains("client_name"));
        let grants = vec!["authorization_code"; 5];
        let (status, answer) =
            register(&app, json!({"redirect_uris": [REDIRECT], "grant_types": grants})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(answer["error"], "invalid_client_metadata");
        // Thân quá 16 KiB.
        let padding = "p".repeat(17 * 1024);
        let (status, answer) =
            register(&app, json!({"redirect_uris": [REDIRECT], "software_id": padding})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(answer["error"], "invalid_client_metadata");
        let (status, answer) =
            form(&app, "/oauth/token", &[("grant_type", "refresh_token"), ("pad", &padding)]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(answer["error"], "invalid_request");
        let (status, answer) = form(&app, "/oauth/revoke", &[("token", &padding)]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(answer["error"], "invalid_request");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hai_lan_doi_cung_refresh_token_dong_thoi_chi_mot_lan_thang() {
    TestDatabase::run(async |test| {
        let (app, session) = oauth_app(test).await;
        let (client_id, code) = approved_code(&app, &session, &["notes.write"]).await;
        let (_, tokens) = exchange(&app, &client_id, &code, pkce_verifier()).await;
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
        // Lượt thua là "dùng lại" (không phân biệt được với kẻ trộm): grant bị thu hồi, token của lượt thắng cũng ngừng.
        let won = if first.0 == StatusCode::OK { first.1 } else { second.1 };
        let access = won["access_token"].as_str().unwrap();
        assert_eq!(mcp_status(&app, access).await, StatusCode::UNAUTHORIZED);
    })
    .await
    .unwrap();
}

/// Mở `/authorize` cho `client_id`, trả token của yêu cầu đang chờ.
async fn request_token(app: &TestApp, client_id: &str, redirect: &str) -> String {
    let challenge = pkce_challenge();
    let authorize = format!(
        "/api/v1/auth/oauth/authorize?response_type=code&client_id={}&redirect_uri={}&code_challenge={challenge}\
         &code_challenge_method=S256",
        encode(client_id),
        encode(redirect),
    );
    let answer = app.get(&authorize).send().await.unwrap();
    assert_eq!(answer.status(), StatusCode::FOUND, "{}", answer.text());
    query_of(answer.header(header::LOCATION))["request"].clone()
}

#[tokio::test]
async fn duyet_quyen_nhay_cam_can_xac_nhan_lai_va_chi_quyen_duoc_moi() {
    TestDatabase::run(async |test| {
        let (app, session) = oauth_app(test).await;
        let registration = json!({"redirect_uris": [REDIRECT], "client_name": "Test client"});
        let (_, _, client) = call(&app, "POST", "/oauth/register", None, Some(registration)).await;
        let client_id = client["client_id"].as_str().unwrap().to_owned();
        let token = request_token(&app, &client_id, REDIRECT).await;
        let approve = format!("/oauth/requests/{token}/approve");
        // Permission phá huỷ (`auth.oauth_grants.manage`) cần `reauth_token`; tên lạ không được mời.
        let decision = json!({"permissions": ["auth.oauth_grants.manage", "nope.read"]});
        let (status, _, problem) =
            call(&app, "POST", &approve, Some(&session), Some(decision)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        let codes: Vec<&str> = problem["errors"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|error| error["code"].as_str())
            .collect();
        assert!(codes.contains(&"not_offered") && codes.contains(&"required"), "{problem}");
        // Yêu cầu vẫn chờ (lỗi đầu vào không đốt nó).
        let (status, _, _) =
            call(&app, "GET", &format!("/oauth/requests/{token}"), Some(&session), None).await;
        assert_eq!(status, StatusCode::OK);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn api_key_va_token_oauth_khong_goi_duoc_api_consent() {
    TestDatabase::run(async |test| {
        let (app, session) = oauth_app(test).await;
        let body =
            json!({"name": "robot", "current_password": PASSWORD, "permissions": ["notes.write"]});
        let (status, _, created) =
            call(&app, "POST", "/api-keys", Some(&session), Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let key = format!("Bearer {}", created["data"]["key"].as_str().unwrap());
        let registration = json!({"redirect_uris": [REDIRECT], "client_name": "Test client"});
        let (_, _, client) = call(&app, "POST", "/oauth/register", None, Some(registration)).await;
        let client_id = client["client_id"].as_str().unwrap().to_owned();
        let token = request_token(&app, &client_id, REDIRECT).await;
        let (status, _, problem) =
            call(&app, "GET", &format!("/oauth/requests/{token}"), Some(&key), None).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("auth.session_required"))
        );
        let decision = json!({"permissions": ["notes.write"]});
        let (status, _, _) = call(
            &app,
            "POST",
            &format!("/oauth/requests/{token}/approve"),
            Some(&key),
            Some(decision),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        // Token OAuth (cấp cho `/mcp`) không tới được REST của consent.
        let (client_id, code) = approved_code(&app, &session, &["notes.write"]).await;
        let (_, tokens) = exchange(&app, &client_id, &code, pkce_verifier()).await;
        let oauth = format!("Bearer {}", tokens["access_token"].as_str().unwrap());
        let (status, _, _) = call(&app, "GET", "/me/oauth-grants", Some(&oauth), None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dang_xuat_khap_noi_thu_hoi_grant_va_token_sai_khach_bi_tu_choi() {
    TestDatabase::run(async |test| {
        let (app, session) = oauth_app(test).await;
        let (client_id, code) = approved_code(&app, &session, &["notes.write"]).await;
        let (_, tokens) = exchange(&app, &client_id, &code, pkce_verifier()).await;
        let access = tokens["access_token"].as_str().unwrap().to_owned();
        assert_eq!(mcp_status(&app, &access).await, StatusCode::OK);
        // Đổi ID khách trong token: không khớp băm, không lộ khách nào tồn tại.
        let forged = format!("axo1_{}{}", uuid::Uuid::now_v7().simple(), &access[37..]);
        assert_eq!(mcp_status(&app, &forged).await, StatusCode::UNAUTHORIZED);
        let (status, _, _) = call(&app, "DELETE", "/me/sessions", Some(&session), None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(mcp_status(&app, &access).await, StatusCode::UNAUTHORIZED);
        // Đăng ký động: `redirect_uri` http tới máy khác bị từ chối.
        let registration = json!({"redirect_uris": ["http://evil.example/cb"]});
        let (status, _, error) =
            call(&app, "POST", "/oauth/register", None, Some(registration)).await;
        assert_eq!(
            (status, error["error"].as_str()),
            (StatusCode::BAD_REQUEST, Some("invalid_redirect_uri"))
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn client_nhan_dien_bang_tai_lieu_metadata() {
    TestDatabase::run(async |test| {
        let (app, session) = oauth_app(test).await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client_id =
            format!("http://127.0.0.1:{}/client.json", listener.local_addr().unwrap().port());
        let document = json!({
            "client_id": client_id,
            "client_name": "Metadata client",
            "redirect_uris": [REDIRECT],
            "token_endpoint_auth_method": "none",
        });
        // Máy chủ tài liệu tối giản: trả cùng một tài liệu JSON cho mọi request.
        let body = document.to_string();
        let server = tokio::spawn(async move {
            use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut buffer = [0u8; 4096];
                let _ = stream.read(&mut buffer).await;
                let answer = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(answer.as_bytes()).await;
            }
        });
        let token = request_token(&app, &client_id, "http://127.0.0.1:40000/cb").await;
        let (status, _, request) =
            call(&app, "GET", &format!("/oauth/requests/{token}"), Some(&session), None).await;
        assert_eq!(status, StatusCode::OK, "{request}");
        assert_eq!(request["data"]["registration"], "cimd");
        assert_eq!(request["data"]["client_name"], "Metadata client");
        assert_eq!(request["data"]["client_host"], "127.0.0.1");
        let decision = json!({"permissions": ["notes.write"]});
        let (status, _, approved) = call(
            &app,
            "POST",
            &format!("/oauth/requests/{token}/approve"),
            Some(&session),
            Some(decision),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{approved}");
        let code = query_of(approved["data"]["redirect_to"].as_str().unwrap())["code"].clone();
        let (status, tokens) = form(
            &app,
            "/oauth/token",
            &[
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("code_verifier", pkce_verifier()),
                ("client_id", &client_id),
                ("redirect_uri", "http://127.0.0.1:40000/cb"),
            ],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{tokens}");
        server.abort();
    })
    .await
    .unwrap();
}

/// Token OAuth thật (cấp cho REST) không quản lý được API key hay passkey của người dùng: không thấy key nào, thu hồi
/// thì 404, xoay thì 403, passkey cần phiên đăng nhập (#572, D-20261126-mcp-oauth).
#[tokio::test]
async fn token_oauth_that_khong_quan_ly_duoc_api_key_va_passkey() {
    TestDatabase::run(async |test| {
        let (app, session) = oauth_app(test).await;
        let body =
            json!({"name": "robot", "current_password": PASSWORD, "permissions": ["notes.write"]});
        let (status, _, created) =
            call(&app, "POST", "/api-keys", Some(&session), Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let key_id = created["data"]["id"].as_str().unwrap().to_owned();

        let rest = format!("{PUBLIC}/api/v1");
        let (client_id, code) = approved_code_for(&app, &session, &["notes.write"], &rest).await;
        let (status, tokens) = exchange_for(&app, &client_id, &code, pkce_verifier(), &rest).await;
        assert_eq!(status, StatusCode::OK, "{tokens}");
        let oauth = format!("Bearer {}", tokens["access_token"].as_str().unwrap());

        let (status, _, listed) = call(&app, "GET", "/api-keys", Some(&oauth), None).await;
        assert_eq!(status, StatusCode::OK, "{listed}");
        assert_eq!(listed["data"], json!([]));
        let (status, _, _) =
            call(&app, "DELETE", &format!("/api-keys/{key_id}"), Some(&oauth), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let rotate = json!({"current_password": PASSWORD});
        let (status, _, problem) =
            call(&app, "POST", &format!("/api-keys/{key_id}/rotate"), Some(&oauth), Some(rotate))
                .await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("auth.api_key_cannot_create_keys")),
            "{problem}"
        );
        let (status, _, problem) = call(&app, "GET", "/me/passkeys", Some(&oauth), None).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("auth.session_required"))
        );
        // Key vẫn còn với chủ của nó.
        let (_, _, mine) = call(&app, "GET", "/api-keys", Some(&session), None).await;
        assert_eq!(mine["data"].as_array().unwrap().len(), 1);
    })
    .await
    .unwrap();
}
