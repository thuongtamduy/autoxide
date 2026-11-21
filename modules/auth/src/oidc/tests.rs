//! Đăng nhập qua nhà cung cấp danh tính (bước 40 P3) qua REST, với máy chủ OIDC giả chạy trong test
//! (axum trên 127.0.0.1): discovery, JWKS, token endpoint, API kiểu GitHub. `id_token` ký ES256 bằng
//! khoá sinh lúc chạy — không có khoá riêng nào trong repo. RS256 kiểm bằng ví dụ RFC 7515 ở `jwt`.

use autoxide_kernel::time::Timestamp;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use autoxide_kernel::event::Actor;
use autoxide_kernel::mail::Mailer;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::body::Bytes;
use axum::http::{Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use p256::ecdsa::SigningKey;
use p256::ecdsa::signature::Signer as _;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::http::test_support::PASSWORD;
use crate::testing::Services;
use crate::user::NewUser;

const REDIRECT: &str = "https://app.example.com/sign-in/callback";
const CLIENT_ID: &str = "autoxide-test";

/// Một `code` đang chờ đổi: `nonce` và PKCE challenge lấy từ `authorization_url`.
struct Pending {
    nonce: String,
    challenge: String,
    claims: Value,
}

/// Máy chủ OIDC giả.
#[derive(Default)]
struct Fake {
    base: OnceLock<String>,
    /// Khoá công bố trong JWKS.
    published: Mutex<Vec<(String, SigningKey)>>,
    /// Khoá ký `id_token` (kid, khoá).
    signer: Mutex<Option<(String, SigningKey)>>,
    jwks_fetches: AtomicUsize,
    codes: Mutex<HashMap<String, Pending>>,
    /// Hồ sơ GitHub: `user`, `emails`.
    github: Mutex<Value>,
}

fn new_key() -> SigningKey {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).unwrap();
    SigningKey::from_slice(&bytes).unwrap()
}

impl Fake {
    fn base(&self) -> &str {
        self.base.get().unwrap()
    }

    /// Khoá mới `kid`, dùng để ký; `publish`: có trong JWKS ngay.
    fn rotate(&self, kid: &str, publish: bool) {
        let key = new_key();
        if publish {
            self.published.lock().unwrap().push((kid.to_owned(), key.clone()));
        }
        *self.signer.lock().unwrap() = Some((kid.to_owned(), key));
    }

    fn publish_signer(&self) {
        let signer = self.signer.lock().unwrap().clone().unwrap();
        self.published.lock().unwrap().push(signer);
    }

    fn jwks(&self) -> Value {
        let keys: Vec<Value> = self
            .published
            .lock()
            .unwrap()
            .iter()
            .map(|(kid, key)| {
                let point = key.verifying_key().to_encoded_point(false);
                json!({
                    "kty": "EC", "crv": "P-256", "kid": kid, "use": "sig", "alg": "ES256",
                    "x": URL_SAFE_NO_PAD.encode(point.x().unwrap()),
                    "y": URL_SAFE_NO_PAD.encode(point.y().unwrap()),
                })
            })
            .collect();
        json!({ "keys": keys })
    }

    fn sign(&self, claims: &Value) -> String {
        let alg = claims.get("__alg").and_then(Value::as_str).unwrap_or("ES256").to_owned();
        let mut claims = claims.clone();
        claims.as_object_mut().unwrap().remove("__alg");
        let (kid, key) = self.signer.lock().unwrap().clone().unwrap();
        let header = json!({"alg": alg, "kid": kid});
        let input = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(header.to_string()),
            URL_SAFE_NO_PAD.encode(claims.to_string())
        );
        if alg != "ES256" {
            return format!("{input}.");
        }
        let signature: p256::ecdsa::Signature = key.sign(input.as_bytes());
        format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature.to_bytes()))
    }
}

fn form(body: &[u8]) -> HashMap<String, String> {
    let text = String::from_utf8_lossy(body).replace('+', " ");
    text.split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(name, value)| (decode(name), decode(value)))
        .collect()
}

fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 3 <= bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap();
            out.push(u8::from_str_radix(hex, 16).unwrap());
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).unwrap()
}

fn json_response(status: StatusCode, body: Value) -> Response {
    let mut response = (status, axum::Json(body)).into_response();
    response.headers_mut().insert("cache-control", "max-age=3600".parse().unwrap());
    response
}

async fn answer(
    axum::extract::State(fake): axum::extract::State<Arc<Fake>>,
    method: Method,
    uri: Uri,
    body: Bytes,
) -> Response {
    let path = uri.path().to_owned();
    let base = fake.base().to_owned();
    if let Some(prefix) = path.strip_suffix("/.well-known/openid-configuration") {
        // Microsoft đa tenant: issuer trong discovery là mẫu `{tenantid}`.
        let issuer = if prefix.ends_with("/organizations/v2.0") {
            format!("{base}{}", prefix.replace("/organizations/", "/{tenantid}/"))
        } else {
            format!("{base}{prefix}")
        };
        return json_response(
            StatusCode::OK,
            json!({
                "issuer": issuer,
                "authorization_endpoint": format!("{base}{prefix}/authorize"),
                "token_endpoint": format!("{base}{prefix}/token"),
                "jwks_uri": format!("{base}{prefix}/jwks"),
                "token_endpoint_auth_methods_supported": ["client_secret_post"],
            }),
        );
    }
    if path.ends_with("/jwks") {
        fake.jwks_fetches.fetch_add(1, Ordering::SeqCst);
        return json_response(StatusCode::OK, fake.jwks());
    }
    if method == Method::POST && (path.ends_with("/token") || path.ends_with("/access_token")) {
        let fields = form(&body);
        let code = fields.get("code").cloned().unwrap_or_default();
        let Some(pending) = fake.codes.lock().unwrap().remove(&code) else {
            return json_response(StatusCode::BAD_REQUEST, json!({"error": "invalid_grant"}));
        };
        // PKCE: băm `code_verifier` phải bằng challenge đã gửi lúc bắt đầu.
        let verifier = fields.get("code_verifier").cloned().unwrap_or_default();
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        if challenge != pending.challenge || !fields.contains_key("client_secret") {
            return json_response(StatusCode::BAD_REQUEST, json!({"error": "invalid_grant"}));
        }
        if path.ends_with("/access_token") {
            return json_response(StatusCode::OK, json!({"access_token": format!("gh-{code}")}));
        }
        let mut claims = pending.claims;
        if claims.get("nonce").is_none() {
            claims["nonce"] = json!(pending.nonce);
        }
        return json_response(StatusCode::OK, json!({"id_token": fake.sign(&claims)}));
    }
    if path.ends_with("/api/v3/user") {
        return json_response(StatusCode::OK, fake.github.lock().unwrap()["user"].clone());
    }
    if path.ends_with("/api/v3/user/emails") {
        return json_response(StatusCode::OK, fake.github.lock().unwrap()["emails"].clone());
    }
    json_response(StatusCode::NOT_FOUND, json!({"error": "not_found"}))
}

async fn fake_server() -> Arc<Fake> {
    let fake = Arc::new(Fake::default());
    fake.rotate("k1", true);
    let service = axum::handler::Handler::with_state(answer, fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    fake.base.set(format!("http://{address}")).unwrap();
    tokio::spawn(async move { axum::serve(listener, service.into_make_service()).await });
    fake
}

/// App có OIDC bật (mạng riêng cho phép, vì máy chủ giả ở 127.0.0.1), một quản trị viên
/// `root@example.com` và `ada@example.com` (email chưa xác minh).
async fn app(test: &TestDatabase, private: bool) -> (TestApp, Services, String) {
    let services = crate::testing::services(test).await;
    let mailer: Arc<dyn Mailer> = services.mails.clone();
    let app = TestApp::builder(test)
        .setting("AUTOXIDE_AUTH_TOKEN_SECRET", crate::testing::TOKEN_SECRET)
        .setting("AUTOXIDE_AUTH_OIDC", "true")
        .setting("AUTOXIDE_AUTH_OIDC_ALLOW_PRIVATE_NETWORKS", &private.to_string())
        .provide::<dyn Mailer>(mailer)
        .module(crate::module())
        .build()
        .await
        .unwrap();
    let tenant = services.tenant.clone();
    let root = NewUser {
        email: "root@example.com".to_owned(),
        name: "Root".to_owned(),
        password: Some(PASSWORD.to_owned()),
    };
    services.users.create_with_roles(&tenant, Actor::system(), root, true).await.unwrap();
    let ada = NewUser {
        email: "ada@example.com".to_owned(),
        name: "Ada".to_owned(),
        password: Some(PASSWORD.to_owned()),
    };
    services.users.create(&tenant, Actor::system(), ada).await.unwrap();
    let admin = token(&app, "root@example.com").await;
    (app, services, admin)
}

async fn token(app: &TestApp, email: &str) -> String {
    let body = json!({"email": email, "password": PASSWORD});
    let response = app.post("/api/v1/auth/login").json(body).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK, "{}", response.json());
    response.json()["data"]["access_token"].as_str().unwrap().to_owned()
}

/// Tạo nhà cung cấp qua REST; trả thân response.
async fn provider(app: &TestApp, admin: &str, body: Value) -> Value {
    let response = app.post("/api/v1/auth/oidc-providers").token(admin).json(body).send().await;
    let response = response.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED, "{}", response.json());
    response.json()["data"].clone()
}

fn oidc_body(fake: &Fake, slug: &str, signup: bool) -> Value {
    json!({
        "slug": slug, "kind": "oidc", "name": "Test ID", "issuer": format!("{}/{slug}", fake.base()),
        "client_id": CLIENT_ID, "client_secret": "s3cret", "redirect_uris": [REDIRECT],
        "allow_signup": signup,
    })
}

fn claims(fake: &Fake, slug: &str, subject: &str, email: &str, verified: bool) -> Value {
    let now = Timestamp::now().as_unix_seconds();
    json!({
        "iss": format!("{}/{slug}", fake.base()), "aud": CLIENT_ID, "sub": subject,
        "email": email, "email_verified": verified, "name": "Grace Hopper",
        "iat": now, "exp": now + 300,
    })
}

/// `POST /oidc/{slug}/start`: trả `state`, `nonce`, challenge trong `authorization_url`.
async fn start(app: &TestApp, slug: &str, tenant: Option<&str>) -> HashMap<String, String> {
    let mut request = app.post(&format!("/api/v1/auth/oidc/{slug}/start"));
    if let Some(tenant) = tenant {
        request = request.header("autoxide-tenant", tenant);
    }
    let response = request.json(json!({"redirect_uri": REDIRECT})).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK, "{}", response.json());
    let url = response.json()["data"]["authorization_url"].as_str().unwrap().to_owned();
    let url = reqwest::Url::parse(&url).unwrap();
    url.query_pairs().map(|(name, value)| (name.into_owned(), value.into_owned())).collect()
}

/// Nhà cung cấp "chấp nhận" lượt `query` với `claims`; trả `code`.
fn approve(fake: &Fake, query: &HashMap<String, String>, claims: Value) -> String {
    let code = format!("code-{}", uuid::Uuid::now_v7());
    let pending = Pending {
        nonce: query["nonce"].clone(),
        challenge: query["code_challenge"].clone(),
        claims,
    };
    fake.codes.lock().unwrap().insert(code.clone(), pending);
    code
}

async fn callback(app: &TestApp, slug: &str, code: &str, state: &str) -> (StatusCode, Value) {
    let response = app
        .post(&format!("/api/v1/auth/oidc/{slug}/callback"))
        .json(json!({"code": code, "state": state}))
        .send()
        .await
        .unwrap();
    (response.status(), response.json())
}

/// Cả lượt đăng nhập với `claims`.
async fn sign_in(app: &TestApp, fake: &Fake, slug: &str, claims: Value) -> (StatusCode, Value) {
    let query = start(app, slug, None).await;
    let code = approve(fake, &query, claims);
    callback(app, slug, &code, &query["state"]).await
}

async fn me(app: &TestApp, access: &str) -> Value {
    let response = app.get("/api/v1/auth/me").token(access).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK, "{}", response.json());
    response.json()["data"].clone()
}

#[tokio::test]
async fn dang_nhap_lan_dau_tao_tai_khoan_khi_cho_phep() {
    TestDatabase::run(async |test| {
        let fake = fake_server().await;
        let (app, _, admin) = app(test, true).await;
        provider(&app, &admin, oidc_body(&fake, "closed", false)).await;
        // Không cho đăng ký: không ai có danh tính này thì từ chối.
        let refused = sign_in(
            &app,
            &fake,
            "closed",
            claims(&fake, "closed", "s-1", "grace@example.com", true),
        );
        let (status, body) = refused.await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("auth.oidc_signup_closed"))
        );

        // Cho đăng ký nhưng đăng ký nhân sự đóng và không có role mặc định: vẫn đóng.
        provider(&app, &admin, oidc_body(&fake, "open", true)).await;
        let (status, body) =
            sign_in(&app, &fake, "open", claims(&fake, "open", "s-1", "grace@example.com", true))
                .await;
        assert_eq!(body["code"], "auth.oidc_signup_closed", "{status}");

        // Có role mặc định: quản trị chủ động mở cửa cho người của nhà cung cấp này.
        let roles = app.get("/api/v1/auth/roles").token(&admin).send().await.unwrap().json();
        let role = roles["data"].as_array().unwrap()[0]["id"].clone();
        let mut body = oidc_body(&fake, "staff", true);
        body["default_role_id"] = role;
        body["allowed_email_domains"] = json!(["example.com"]);
        provider(&app, &admin, body).await;
        let (status, body) =
            sign_in(&app, &fake, "staff", claims(&fake, "staff", "s-1", "Grace@Example.com", true))
                .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let access = body["data"]["access_token"].as_str().unwrap().to_owned();
        let account = me(&app, &access).await;
        assert_eq!(account["email"], "grace@example.com");
        assert_eq!(account["name"], "Grace Hopper");
        assert!(account["email_verified_at"].is_string(), "{account}");
        // Lần sau: cùng tài khoản qua liên kết, không tạo mới.
        let (status, again) =
            sign_in(&app, &fake, "staff", claims(&fake, "staff", "s-1", "grace@example.com", true))
                .await;
        assert_eq!(status, StatusCode::OK, "{again}");
        let access = again["data"]["access_token"].as_str().unwrap();
        assert_eq!(me(&app, access).await["id"], account["id"]);
        // Tên miền ngoài danh sách.
        let (status, body) =
            sign_in(&app, &fake, "staff", claims(&fake, "staff", "s-2", "eve@evil.test", true))
                .await;
        assert_eq!(body["code"], "auth.oidc_email_domain_not_allowed", "{status}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn lien_ket_theo_email_da_xac_minh() {
    TestDatabase::run(async |test| {
        let fake = fake_server().await;
        let (app, services, admin) = app(test, true).await;
        provider(&app, &admin, oidc_body(&fake, "idp", false)).await;
        // `root@example.com` đã xác minh email trong Autoxide.
        let tenant = services.tenant.clone();
        let root =
            services.users.find_by_email(&tenant, "root@example.com").await.unwrap().unwrap();
        let transaction = tenant.database().begin_write().await.unwrap();
        services
            .users
            .apply_verified_email(&tenant, &transaction, root.id, &root.email, Timestamp::now())
            .await
            .unwrap();
        transaction.commit().await.unwrap();
        let (status, body) =
            sign_in(&app, &fake, "idp", claims(&fake, "idp", "s-9", "root@example.com", true))
                .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let access = body["data"]["access_token"].as_str().unwrap();
        assert_eq!(me(&app, access).await["id"], json!(root.id));
        let response = app.get("/api/v1/auth/me/identities").token(access).send().await.unwrap();
        let linked = response.json()["data"].clone();
        assert_eq!(linked.as_array().unwrap().len(), 1, "{linked}");
        assert_eq!(linked[0]["provider"], "idp");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khong_gop_khi_email_chua_xac_minh() {
    TestDatabase::run(async |test| {
        let fake = fake_server().await;
        let (app, services, admin) = app(test, true).await;
        provider(&app, &admin, oidc_body(&fake, "idp", true)).await;
        // Phía Autoxide chưa xác minh (`ada` tạo sẵn), nhà cung cấp đã xác minh: không gộp.
        let (status, body) =
            sign_in(&app, &fake, "idp", claims(&fake, "idp", "s-1", "ada@example.com", true)).await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::CONFLICT, Some("auth.oidc_link_required"))
        );
        // Phía nhà cung cấp chưa xác minh, Autoxide đã xác minh: cũng không gộp.
        let tenant = services.tenant.clone();
        let root =
            services.users.find_by_email(&tenant, "root@example.com").await.unwrap().unwrap();
        let transaction = tenant.database().begin_write().await.unwrap();
        let now = Timestamp::now();
        services
            .users
            .apply_verified_email(&tenant, &transaction, root.id, &root.email, now)
            .await
            .unwrap();
        transaction.commit().await.unwrap();
        let (_, body) =
            sign_in(&app, &fake, "idp", claims(&fake, "idp", "s-2", "root@example.com", false))
                .await;
        assert_eq!(body["code"], "auth.oidc_link_required");
        // Không ai có email đó và nhà cung cấp chưa xác minh: không tạo tài khoản.
        let (_, body) =
            sign_in(&app, &fake, "idp", claims(&fake, "idp", "s-3", "new@example.com", false))
                .await;
        assert_eq!(body["code"], "auth.oidc_email_unverified");
        assert!(crate::oidc::identity::for_user(&tenant, root.id).await.unwrap().is_empty());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn microsoft_khong_tin_claim_email_khi_thieu_xms_edov() {
    TestDatabase::run(async |test| {
        let fake = fake_server().await;
        let (app, services, admin) = app(test, true).await;
        let issuer = format!("{}/ms/organizations/v2.0", fake.base());
        let body = json!({
            "slug": "entra", "kind": "microsoft", "name": "Microsoft", "issuer": issuer,
            "client_id": CLIENT_ID, "client_secret": "s3cret", "redirect_uris": [REDIRECT],
        });
        provider(&app, &admin, body).await;
        let tenant = services.tenant.clone();
        let root =
            services.users.find_by_email(&tenant, "root@example.com").await.unwrap().unwrap();
        let transaction = tenant.database().begin_write().await.unwrap();
        let now = Timestamp::now();
        services
            .users
            .apply_verified_email(&tenant, &transaction, root.id, &root.email, now)
            .await
            .unwrap();
        transaction.commit().await.unwrap();
        let tid = "7c9e6679-7425-40de-944b-e07fc1f90ae7";
        let entra = |verified: Option<bool>| {
            let mut claims = claims(&fake, "x", "entra-oid", "root@example.com", true);
            claims["iss"] = json!(format!("{}/ms/{tid}/v2.0", fake.base()));
            claims["tid"] = json!(tid);
            if let Some(verified) = verified {
                claims["xms_edov"] = json!(verified);
            }
            claims
        };
        // `email_verified = true` nhưng thiếu `xms_edov`: không tin email của Entra.
        let (_, body) = sign_in(&app, &fake, "entra", entra(None)).await;
        assert_eq!(body["code"], "auth.oidc_link_required", "{body}");
        let (_, body) = sign_in(&app, &fake, "entra", entra(Some(false))).await;
        assert_eq!(body["code"], "auth.oidc_link_required", "{body}");
        let (status, body) = sign_in(&app, &fake, "entra", entra(Some(true))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        // `iss` của tenant khác `tid`: sai issuer.
        let mut forged = entra(Some(true));
        forged["iss"] =
            json!(format!("{}/ms/00000000-0000-0000-0000-000000000000/v2.0", fake.base()));
        let (_, body) = sign_in(&app, &fake, "entra", forged).await;
        assert_eq!(
            (body["code"].as_str(), body["reason"].as_str()),
            (Some("auth.oidc_token_invalid"), Some("issuer"))
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn state_dung_mot_lan() {
    TestDatabase::run(async |test| {
        let fake = fake_server().await;
        let (app, services, admin) = app(test, true).await;
        provider(&app, &admin, oidc_body(&fake, "idp", false)).await;
        let tenant = services.tenant.clone();
        let root =
            services.users.find_by_email(&tenant, "root@example.com").await.unwrap().unwrap();
        let transaction = tenant.database().begin_write().await.unwrap();
        let now = Timestamp::now();
        services
            .users
            .apply_verified_email(&tenant, &transaction, root.id, &root.email, now)
            .await
            .unwrap();
        transaction.commit().await.unwrap();
        let query = start(&app, "idp", None).await;
        let code = approve(&fake, &query, claims(&fake, "idp", "s-1", "root@example.com", true));
        let state = query["state"].clone();
        let (first, second) = tokio::join!(
            callback(&app, "idp", &code, &state),
            callback(&app, "idp", &code, &state)
        );
        let statuses = [first.0, second.0];
        assert_eq!(
            statuses.iter().filter(|status| **status == StatusCode::OK).count(),
            1,
            "{first:?} {second:?}"
        );
        let failed = if first.0 == StatusCode::OK { second.1 } else { first.1 };
        assert_eq!(failed["code"], "auth.oidc_state_invalid");
        // `state` lạ.
        let (status, body) = callback(&app, "idp", &code, "not-a-state").await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::BAD_REQUEST, Some("auth.oidc_state_invalid"))
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nonce_aud_iss_exp_alg_sai_bi_tu_choi() {
    TestDatabase::run(async |test| {
        let fake = fake_server().await;
        let (app, _, admin) = app(test, true).await;
        provider(&app, &admin, oidc_body(&fake, "idp", true)).await;
        let now = Timestamp::now().as_unix_seconds();
        let cases = [
            ("nonce", json!("someone-else"), "nonce"),
            ("aud", json!("other-client"), "audience"),
            ("iss", json!("https://evil.example"), "issuer"),
            ("exp", json!(now - 3600), "expired"),
            ("__alg", json!("none"), "algorithm"),
            ("__alg", json!("HS256"), "algorithm"),
        ];
        for (name, value, reason) in cases {
            let mut wrong = claims(&fake, "idp", "s-1", "x@example.com", true);
            wrong[name] = value;
            let (status, body) = sign_in(&app, &fake, "idp", wrong).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{name}: {body}");
            assert_eq!(body["code"], "auth.oidc_token_invalid");
            assert_eq!(body["reason"], reason, "{name}");
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn kid_la_thi_tai_lai_jwks_mot_lan() {
    TestDatabase::run(async |test| {
        let fake = fake_server().await;
        let (app, services, admin) = app(test, true).await;
        provider(&app, &admin, oidc_body(&fake, "idp", false)).await;
        let tenant = services.tenant.clone();
        let root =
            services.users.find_by_email(&tenant, "root@example.com").await.unwrap().unwrap();
        let transaction = tenant.database().begin_write().await.unwrap();
        let now = Timestamp::now();
        services
            .users
            .apply_verified_email(&tenant, &transaction, root.id, &root.email, now)
            .await
            .unwrap();
        transaction.commit().await.unwrap();
        let good = || claims(&fake, "idp", "s-1", "root@example.com", true);
        assert_eq!(sign_in(&app, &fake, "idp", good()).await.0, StatusCode::OK);
        assert_eq!(fake.jwks_fetches.load(Ordering::SeqCst), 1);
        // Nhà cung cấp xoay khoá nhưng chưa công bố: tải lại đúng một lần, rồi không tải nữa.
        fake.rotate("k2", false);
        let (_, body) = sign_in(&app, &fake, "idp", good()).await;
        assert_eq!(body["reason"], "unknown_key", "{body}");
        let (_, body) = sign_in(&app, &fake, "idp", good()).await;
        assert_eq!(body["reason"], "unknown_key", "{body}");
        assert_eq!(fake.jwks_fetches.load(Ordering::SeqCst), 2, "tối đa một lần mỗi phút");
        // Khoá cũ vẫn trong cache: không tải lại.
        fake.publish_signer();
        fake.rotate("k1-again", false);
        assert_eq!(fake.jwks_fetches.load(Ordering::SeqCst), 2);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn mfa_van_ap_dung() {
    TestDatabase::run(async |test| {
        let fake = fake_server().await;
        let (app, services, admin) = app(test, true).await;
        provider(&app, &admin, oidc_body(&fake, "idp", false)).await;
        let tenant = services.tenant.clone();
        let root =
            services.users.find_by_email(&tenant, "root@example.com").await.unwrap().unwrap();
        let transaction = tenant.database().begin_write().await.unwrap();
        let now = Timestamp::now();
        services
            .users
            .apply_verified_email(&tenant, &transaction, root.id, &root.email, now)
            .await
            .unwrap();
        transaction.commit().await.unwrap();
        let setup = services.mfa.start_totp(&tenant, root.id, "root@example.com").await.unwrap();
        let seconds = u64::try_from(Timestamp::now().as_unix_seconds()).unwrap();
        let code = crate::mfa::service::tests::code_at(&setup.secret, seconds);
        services.mfa.confirm_totp(&tenant, root.id, &code).await.unwrap();
        let (status, body) =
            sign_in(&app, &fake, "idp", claims(&fake, "idp", "s-1", "root@example.com", true))
                .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        assert!(body["data"]["mfa_token"].is_string(), "{body}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn redirect_uri_ngoai_danh_sach_va_tat_mac_dinh() {
    TestDatabase::run(async |test| {
        let fake = fake_server().await;
        let (app, _, admin) = app(test, true).await;
        provider(&app, &admin, oidc_body(&fake, "idp", false)).await;
        for uri in [
            "https://app.example.com/sign-in/callback/x",
            "https://app.example.com.evil.test/sign-in/callback",
        ] {
            let response = app
                .post("/api/v1/auth/oidc/idp/start")
                .json(json!({"redirect_uri": uri}))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY, "{uri}");
            assert_eq!(response.json()["code"], "auth.oidc_redirect_not_allowed");
        }
        let response =
            app.post("/api/v1/auth/oidc/nope/start").json(json!({"redirect_uri": REDIRECT})).send();
        assert_eq!(response.await.unwrap().json()["code"], "auth.oidc_provider_not_found");
        // `authorization_url` mang PKCE S256, `state`, `nonce` — `code_verifier` thì không bao giờ.
        let query = start(&app, "idp", None).await;
        assert_eq!(query["code_challenge_method"], "S256");
        assert_eq!(query["client_id"], CLIENT_ID);
        assert!(!query.contains_key("code_verifier"));
    })
    .await
    .unwrap();
    // Mặc định tắt.
    TestDatabase::run(async |test| {
        let (app, _) = crate::http::test_support::app(test).await;
        let response = app.get("/api/v1/auth/oidc/providers").send().await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(response.json()["code"], "auth.oidc_disabled");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn issuer_mang_noi_bo_bi_chan() {
    TestDatabase::run(async |test| {
        let (app, _, admin) = app(test, false).await;
        for issuer in [
            "https://127.0.0.1:8443",
            "https://169.254.169.254",
            "https://localhost",
            "https://[::1]",
            "http://id.example.com",
        ] {
            let body = json!({
                "slug": "x", "kind": "oidc", "name": "X", "issuer": issuer, "client_id": "c",
                "redirect_uris": [REDIRECT],
            });
            let response = app.post("/api/v1/auth/oidc-providers").token(&admin).json(body).send();
            let response = response.await.unwrap();
            assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY, "{issuer}");
            assert_eq!(response.json()["errors"][0]["field"], "issuer", "{}", response.json());
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn quan_tri_nha_cung_cap_khong_tra_secret() {
    TestDatabase::run(async |test| {
        let fake = fake_server().await;
        let (app, _, admin) = app(test, true).await;
        let created = provider(&app, &admin, oidc_body(&fake, "idp", false)).await;
        assert_eq!(created["has_secret"], true);
        assert!(created.get("client_secret").is_none(), "{created}");
        let id = created["id"].as_str().unwrap();
        // Trùng slug.
        let response = app
            .post("/api/v1/auth/oidc-providers")
            .token(&admin)
            .json(oidc_body(&fake, "idp", false))
            .send();
        assert_eq!(response.await.unwrap().json()["code"], "auth.oidc_slug_taken");
        // Người không có `auth.oidc.manage`.
        let ada = token(&app, "ada@example.com").await;
        let response = app.get("/api/v1/auth/oidc-providers").token(&ada).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        // Sửa: tắt thì không còn trong danh sách công khai.
        let response = app
            .request(Method::PATCH, &format!("/api/v1/auth/oidc-providers/{id}"))
            .token(&admin)
            .json(json!({"enabled": false, "name": "Renamed"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{}", response.json());
        assert_eq!(response.json()["data"]["name"], "Renamed");
        let public = app.get("/api/v1/auth/oidc/providers").send().await.unwrap().json();
        assert_eq!(public["data"], json!([]));
        let response =
            app.delete(&format!("/api/v1/auth/oidc-providers/{id}")).token(&admin).send();
        assert_eq!(response.await.unwrap().status(), StatusCode::NO_CONTENT);
        let response = app.get(&format!("/api/v1/auth/oidc-providers/{id}")).token(&admin).send();
        assert_eq!(response.await.unwrap().json()["code"], "auth.oidc_provider_not_found");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn github_lay_email_chinh_da_xac_minh() {
    TestDatabase::run(async |test| {
        let fake = fake_server().await;
        let (app, services, admin) = app(test, true).await;
        let roles = app.get("/api/v1/auth/roles").token(&admin).send().await.unwrap().json();
        let role = roles["data"].as_array().unwrap()[0]["id"].clone();
        // GitHub Enterprise Server giả: `issuer` là địa chỉ gốc.
        let body = json!({
            "slug": "github", "kind": "github", "name": "GitHub", "issuer": fake.base(),
            "client_id": CLIENT_ID, "client_secret": "s3cret", "redirect_uris": [REDIRECT],
            "allow_signup": true, "default_role_id": role,
        });
        provider(&app, &admin, body).await;
        *fake.github.lock().unwrap() = json!({
            "user": {"id": 4242, "login": "octo", "name": null},
            "emails": [
                {"email": "octo@example.com", "primary": true, "verified": false},
                {"email": "other@example.com", "primary": false, "verified": true},
            ],
        });
        // Email chính chưa xác minh: không dùng email phụ thay.
        let (_, body) = sign_in(&app, &fake, "github", json!({})).await;
        assert_eq!(body["code"], "auth.oidc_email_unverified", "{body}");
        fake.github.lock().unwrap()["emails"][0]["verified"] = json!(true);
        let (status, body) = sign_in(&app, &fake, "github", json!({})).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let account = me(&app, body["data"]["access_token"].as_str().unwrap()).await;
        assert_eq!(
            (account["email"].as_str(), account["name"].as_str()),
            (Some("octo@example.com"), Some("octo"))
        );
        let user = uuid::Uuid::parse_str(account["id"].as_str().unwrap()).unwrap();
        let linked = crate::oidc::identity::for_user(&services.tenant, user).await.unwrap();
        assert_eq!(linked[0].subject, "4242");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn go_cach_dang_nhap_cuoi_bi_chan_va_lien_ket_khi_da_dang_nhap() {
    TestDatabase::run(async |test| {
        let fake = fake_server().await;
        let (app, services, admin) = app(test, true).await;
        let roles = app.get("/api/v1/auth/roles").token(&admin).send().await.unwrap().json();
        let mut body = oidc_body(&fake, "idp", true);
        body["default_role_id"] = roles["data"].as_array().unwrap()[0]["id"].clone();
        provider(&app, &admin, body).await;
        provider(&app, &admin, oidc_body(&fake, "second", false)).await;
        // Tài khoản tạo bằng nhà cung cấp: không mật khẩu, không passkey.
        let (_, body) =
            sign_in(&app, &fake, "idp", claims(&fake, "idp", "s-1", "grace@example.com", true))
                .await;
        let access = body["data"]["access_token"].as_str().unwrap().to_owned();
        let linked =
            app.get("/api/v1/auth/me/identities").token(&access).send().await.unwrap().json();
        let id = linked["data"][0]["id"].as_str().unwrap().to_owned();
        let response =
            app.delete(&format!("/api/v1/auth/me/identities/{id}")).token(&access).send();
        let response = response.await.unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(response.json()["code"], "auth.identity_last_method");

        // Liên kết nhà cung cấp thứ hai khi đã đăng nhập (xác nhận bằng token reauth qua mail —
        // tài khoản không có mật khẩu).
        let user = uuid::Uuid::parse_str(me(&app, &access).await["id"].as_str().unwrap()).unwrap();
        let tenant = services.tenant.clone();
        let link_start = async |access: &str| {
            let proof = crate::reauth::Proof::EmailLink;
            let language = autoxide_kernel::i18n::Language::En;
            services.reauth.issue(&tenant, user, proof, language).await.unwrap();
            let reauth_token = crate::testing::last_token(&services);
            let body = json!({"redirect_uri": REDIRECT, "reauth_token": reauth_token});
            let request = app.post("/api/v1/auth/me/identities/second/start").token(access);
            let response = request.json(body).send().await.unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{}", response.json());
            let url = response.json()["data"]["authorization_url"].as_str().unwrap().to_owned();
            reqwest::Url::parse(&url)
                .unwrap()
                .query_pairs()
                .map(|(name, value)| (name.into_owned(), value.into_owned()))
                .collect::<HashMap<String, String>>()
        };
        // Lượt liên kết không dùng được để đăng nhập.
        let query = link_start(&access).await;
        let code =
            approve(&fake, &query, claims(&fake, "second", "z-1", "grace@example.com", true));
        let (status, body) = callback(&app, "second", &code, &query["state"]).await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::BAD_REQUEST, Some("auth.oidc_state_invalid"))
        );
        let query = link_start(&access).await;
        let code =
            approve(&fake, &query, claims(&fake, "second", "z-1", "grace@example.com", true));
        let response = app
            .post("/api/v1/auth/me/identities/second/callback")
            .token(&access)
            .json(json!({"code": code, "state": query["state"]}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED, "{}", response.json());
        assert_eq!(response.json()["data"]["provider"], "second");
        // Còn cách khác: gỡ được.
        let response =
            app.delete(&format!("/api/v1/auth/me/identities/{id}")).token(&access).send();
        assert_eq!(response.await.unwrap().status(), StatusCode::NO_CONTENT);
        let linked =
            app.get("/api/v1/auth/me/identities").token(&access).send().await.unwrap().json();
        assert_eq!(linked["data"].as_array().unwrap().len(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn xuat_va_xoa_identity_cung_tai_khoan() {
    TestDatabase::run(async |test| {
        let fake = fake_server().await;
        let (app, services, admin) = app(test, true).await;
        let roles = app.get("/api/v1/auth/roles").token(&admin).send().await.unwrap().json();
        let mut body = oidc_body(&fake, "idp", true);
        body["default_role_id"] = roles["data"].as_array().unwrap()[0]["id"].clone();
        provider(&app, &admin, body).await;
        let (_, body) =
            sign_in(&app, &fake, "idp", claims(&fake, "idp", "s-1", "grace@example.com", true))
                .await;
        let access = body["data"]["access_token"].as_str().unwrap().to_owned();
        let export = app.get("/api/v1/auth/me/export").token(&access).send().await.unwrap().json();
        let identities = export["data"]["auth"]["identities"].clone();
        assert_eq!(identities[0]["subject"], "s-1", "{export}");
        assert_eq!(identities[0]["email"], "grace@example.com");
        let user = uuid::Uuid::parse_str(me(&app, &access).await["id"].as_str().unwrap()).unwrap();
        let tenant = services.tenant.clone();
        services.users.delete(&tenant, Actor::system(), user).await.unwrap();
        assert!(crate::oidc::identity::for_user(&tenant, user).await.unwrap().is_empty());
        // Danh tính cũ đăng nhập lại: không còn liên kết, coi như người mới.
        let (status, body) =
            sign_in(&app, &fake, "idp", claims(&fake, "idp", "s-1", "grace@example.com", true))
                .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_ne!(
            me(&app, body["data"]["access_token"].as_str().unwrap()).await["id"],
            json!(user)
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn provider_cua_khach_a_khong_dung_duoc_o_khach_b() {
    TestDatabase::run(async |test| {
        let fake = fake_server().await;
        let services = crate::testing::services(test).await;
        let mailer: Arc<dyn Mailer> = services.mails.clone();
        let app = TestApp::builder(test)
            .setting("AUTOXIDE_AUTH_TOKEN_SECRET", crate::testing::TOKEN_SECRET)
            .setting("AUTOXIDE_AUTH_OIDC", "true")
            .setting("AUTOXIDE_AUTH_OIDC_ALLOW_PRIVATE_NETWORKS", "true")
            .tenant("a")
            .tenant("b")
            .provide::<dyn Mailer>(mailer)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let a = app.tenant("a").await.unwrap();
        let root = NewUser {
            email: "root@example.com".to_owned(),
            name: "Root".to_owned(),
            password: Some(PASSWORD.to_owned()),
        };
        services.users.create_with_roles(&a, Actor::system(), root, true).await.unwrap();
        let login = app
            .post("/api/v1/auth/login")
            .header("autoxide-tenant", "a")
            .json(json!({"email": "root@example.com", "password": PASSWORD}))
            .send()
            .await
            .unwrap()
            .json();
        let admin = login["data"]["access_token"].as_str().unwrap().to_owned();
        let response = app
            .post("/api/v1/auth/oidc-providers")
            .header("autoxide-tenant", "a")
            .token(&admin)
            .json(oidc_body(&fake, "idp", true))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED, "{}", response.json());
        // Khách `b` không thấy, không bắt đầu được.
        let public = app.get("/api/v1/auth/oidc/providers").header("autoxide-tenant", "b").send();
        assert_eq!(public.await.unwrap().json()["data"], json!([]));
        let response = app
            .post("/api/v1/auth/oidc/idp/start")
            .header("autoxide-tenant", "b")
            .json(json!({"redirect_uri": REDIRECT}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.json()["code"], "auth.oidc_provider_not_found");
        // `state` của khách `a` mang sang khách `b`: không dùng được.
        let query = start(&app, "idp", Some("a")).await;
        let code = approve(&fake, &query, claims(&fake, "idp", "s-1", "g@example.com", true));
        let response = app
            .post("/api/v1/auth/oidc/idp/callback")
            .header("autoxide-tenant", "b")
            .json(json!({"code": code, "state": query["state"]}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.json()["code"], "auth.oidc_state_invalid");
    })
    .await
    .unwrap();
}

/// Nhà cung cấp do người chỉ có `auth.oidc.manage` cấu hình không tự gộp vào tài khoản có quyền
/// cao hơn người đó; tài khoản không có permission nào thì vẫn gộp theo email đã xác minh.
#[tokio::test]
async fn khong_tu_gop_tai_khoan_quyen_cao_hon_nguoi_cau_hinh() {
    TestDatabase::run(async |test| {
        let fake = fake_server().await;
        let (app, services, _) = app(test, true).await;
        let tenant = services.tenant.clone();
        let input = crate::role::RoleInput {
            denies: Vec::new(),
            requires_mfa: false,
            name: "sso".to_owned(),
            description: String::new(),
            permissions: vec!["auth.oidc.manage".to_owned()],
            grants: Vec::new(),
        };
        let role = services.roles.create(&tenant, Actor::system(), input).await.unwrap();
        let manager = NewUser {
            email: "sso@example.com".to_owned(),
            name: "SSO".to_owned(),
            password: Some(PASSWORD.to_owned()),
        };
        let manager = services.users.create(&tenant, Actor::system(), manager).await.unwrap();
        let root_unit = autoxide_kernel::tenancy::root_of(tenant.id()).as_uuid();
        let boss = autoxide_kernel::access::Principal::with_all_permissions(uuid::Uuid::now_v7())
            .in_tenant(tenant.id());
        services
            .roles
            .set_memberships(&tenant, &boss, manager.id, vec![(root_unit, vec![role.id])])
            .await
            .unwrap();
        let manager_token = token(&app, "sso@example.com").await;
        provider(&app, &manager_token, oidc_body(&fake, "idp", false)).await;
        for email in ["root@example.com", "ada@example.com"] {
            let user = services.users.find_by_email(&tenant, email).await.unwrap().unwrap();
            let transaction = tenant.database().begin_write().await.unwrap();
            services
                .users
                .apply_verified_email(&tenant, &transaction, user.id, email, Timestamp::now())
                .await
                .unwrap();
            transaction.commit().await.unwrap();
        }
        let (status, body) =
            sign_in(&app, &fake, "idp", claims(&fake, "idp", "s-root", "root@example.com", true))
                .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["code"], "auth.oidc_link_required");
        let (status, body) =
            sign_in(&app, &fake, "idp", claims(&fake, "idp", "s-ada", "ada@example.com", true))
                .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    })
    .await
    .unwrap();
}

/// Lượt gắn trình duyệt (`bind_browser`): `code` + `state` không kèm `browser_secret` đúng thì
/// không kết thúc được; kèm đúng thì đăng nhập như thường.
#[tokio::test]
async fn luot_gan_trinh_duyet_doi_browser_secret() {
    TestDatabase::run(async |test| {
        let fake = fake_server().await;
        let (app, services, admin) = app(test, true).await;
        provider(&app, &admin, oidc_body(&fake, "idp", false)).await;
        // Ada (không role) đã xác minh email: đăng nhập lần đầu tự liên kết.
        let tenant = services.tenant.clone();
        let ada = services.users.find_by_email(&tenant, "ada@example.com").await.unwrap().unwrap();
        let transaction = tenant.database().begin_write().await.unwrap();
        services
            .users
            .apply_verified_email(&tenant, &transaction, ada.id, &ada.email, Timestamp::now())
            .await
            .unwrap();
        transaction.commit().await.unwrap();
        let bound = async || {
            let body = json!({"redirect_uri": REDIRECT, "bind_browser": true});
            let response = app.post("/api/v1/auth/oidc/idp/start").json(body).send().await.unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{}", response.json());
            let data = response.json()["data"].clone();
            let url = reqwest::Url::parse(data["authorization_url"].as_str().unwrap()).unwrap();
            let query: HashMap<String, String> =
                url.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
            (query, data["browser_secret"].as_str().unwrap().to_owned())
        };
        let finish = async |code: &str, state: &str, secret: Option<&str>| {
            let mut body = json!({"code": code, "state": state});
            if let Some(secret) = secret {
                body["browser_secret"] = json!(secret);
            }
            let response =
                app.post("/api/v1/auth/oidc/idp/callback").json(body).send().await.unwrap();
            (response.status(), response.json())
        };
        for wrong in [None, Some("not-the-secret")] {
            let (query, _) = bound().await;
            let code = approve(&fake, &query, claims(&fake, "idp", "s-1", "ada@example.com", true));
            let (status, body) = finish(&code, &query["state"], wrong).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
            assert_eq!(body["code"], "auth.oidc_state_invalid");
        }
        let (query, secret) = bound().await;
        let code = approve(&fake, &query, claims(&fake, "idp", "s-1", "ada@example.com", true));
        let (status, body) = finish(&code, &query["state"], Some(&secret)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        // Không xin gắn: như trước, không có `browser_secret`.
        let response = app
            .post("/api/v1/auth/oidc/idp/start")
            .json(json!({"redirect_uri": REDIRECT}))
            .send()
            .await
            .unwrap();
        assert!(response.json()["data"].get("browser_secret").is_none());
    })
    .await
    .unwrap();
}
