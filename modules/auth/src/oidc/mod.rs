//! Đăng nhập qua nhà cung cấp danh tính (bước 40 P3): Google, Microsoft Entra, GitHub (OAuth 2.0)
//! và một issuer OpenID Connect bất kỳ, mỗi khách tự khai của mình. Core chỉ có API nên trình
//! duyệt do giao diện điều khiển:
//!
//! 1. `start`: sinh `state` (token một lần của kernel — tra bằng bản băm, không so chuỗi), `nonce`
//!    và PKCE `code_verifier` (S256). `nonce`, `code_verifier`, `redirect_uri` lưu MÃ HOÁ (AEAD)
//!    trong dòng token, không bao giờ ra khỏi máy chủ; giao diện nhận `authorization_url`.
//! 2. Nhà cung cấp quay về `redirect_uri` của giao diện với `code` + `state`.
//! 3. `callback`: dùng `state` một lần, đổi `code` lấy token (gửi `code_verifier`), kiểm `id_token`
//!    bằng JWKS ([`jwt`]); GitHub không có `id_token`: đọc API `/user` và `/user/emails`.
//! 4. Liên kết tài khoản chỉ khi email đã xác minh ở CẢ HAI phía (chặn chiếm tài khoản dựng sẵn);
//!    tài khoản mới chỉ khi nhà cung cấp cho đăng ký và đăng ký đang mở. MFA của Autoxide vẫn áp
//!    dụng: kết quả như `POST /login`.
//!
//! Gọi nhà cung cấp luôn ở NGOÀI transaction ghi; gọi xong mới mở transaction tạo liên kết.

mod client;
pub(crate) mod identity;
mod jwt;
pub(crate) mod provider;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod unlink_tests;

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::sync::Arc;

use autoxide_kernel::access::Realm;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::{Actor, Outbox};
use autoxide_kernel::secret::{SecretKey, random_token};
use autoxide_kernel::tenancy::{DatabaseRegistry, TenantContext, link_token};
use autoxide_kernel::tokens::OneTimeTokens;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub(crate) use self::client::OidcClient;
use self::identity::{External, Identity};
pub(crate) use self::provider::{Kind, Provider, ProviderService};
use crate::events;
use crate::session::{LoginOutcome, SessionService};
use crate::user::{NewUser, UserService};

/// `purpose` của token `state` trong `auth_tokens`.
const PURPOSE: &str = "oidc_state";
/// Mục đích của lớp mã hoá cho dữ liệu của `state`.
const STATE_PURPOSE: &str = "auth.oidc_state";
/// Hạn của một lượt đăng nhập, giây.
pub(crate) const STATE_TTL_SECONDS: u64 = 600;

const GITHUB_WEB: &str = "https://github.com";
const GITHUB_API: &str = "https://api.github.com";

fn tokens() -> OneTimeTokens {
    OneTimeTokens::new("auth")
}

/// Dữ liệu của một lượt, lưu mã hoá trong dòng token.
#[derive(Serialize, Deserialize)]
struct StateData {
    nonce: String,
    verifier: String,
    redirect_uri: String,
    /// Lượt liên kết từ phần tài khoản (`/me/identities/{slug}/start`): người đang đăng nhập.
    link_user_id: Option<Uuid>,
    /// Bản băm của `browser_secret` khi giao diện xin gắn lượt vào trình duyệt đang dùng
    /// (D-20261121-oidc-browser-binding). Lượt mở trước khi có trường này: vắng.
    #[serde(default)]
    browser: Option<String>,
}

/// Lượt vừa mở.
pub(crate) struct Started {
    pub authorization_url: String,
    pub expires_in: u64,
    /// Chỉ khi xin gắn trình duyệt: gửi lại ở callback.
    pub browser_secret: Option<String>,
}

/// Liên kết kèm nhà cung cấp, cho danh sách của người dùng.
pub(crate) struct LinkedIdentity {
    pub identity: Identity,
    pub provider_slug: String,
    pub provider_name: String,
    pub provider_kind: String,
}

pub(crate) struct OidcService {
    /// `AUTOXIDE_AUTH_OIDC`: tắt thì mọi endpoint đăng nhập trả `auth.oidc_disabled`.
    pub enabled: bool,
    pub providers: ProviderService,
    pub client: OidcClient,
    pub users: Arc<UserService>,
    /// Quyền của người cấu hình nhà cung cấp, so với tài khoản trước khi tự liên kết theo email.
    pub roles: Arc<crate::role::RoleService>,
    pub sessions: Arc<SessionService>,
    pub outbox: Outbox,
    pub secrets: SecretKey,
    pub databases: DatabaseRegistry,
    pub orgs: Option<Arc<dyn autoxide_organizations::api::OrgDirectory>>,
    pub entitlements: autoxide_kernel::entitlement::EntitlementsHandle,
    /// `AUTOXIDE_AUTH_SELF_REGISTRATION`.
    pub self_registration: bool,
}

impl OidcService {
    pub(crate) fn require_enabled(&self) -> Result<(), Error> {
        if self.enabled {
            Ok(())
        } else {
            Err(Error::not_found(
                "auth.oidc_disabled",
                "Signing in with an outside provider is not turned on on this server \
                 (AUTOXIDE_AUTH_OIDC).",
            ))
        }
    }

    /// Nhà cung cấp đang bật của khách, cho nút "Đăng nhập bằng …" của giao diện.
    pub(crate) async fn public_providers(
        &self,
        tenant: &TenantContext,
    ) -> Result<Vec<Provider>, Error> {
        self.require_enabled()?;
        let mut providers = self.providers.list(tenant).await?;
        providers.retain(|provider| provider.enabled);
        Ok(providers)
    }

    /// Mở lượt đăng nhập (hay liên kết, khi `link_user_id`) với nhà cung cấp `slug`. `bind_browser`:
    /// sinh `browser_secret` mà callback phải gửi lại — `state` và `code` lộ ra ngoài (log, lịch
    /// sử trình duyệt) không đủ để kết thúc lượt ở nơi khác.
    pub(crate) async fn start(
        &self,
        tenant: &TenantContext,
        slug: &str,
        redirect_uri: &str,
        link_user_id: Option<Uuid>,
        bind_browser: bool,
    ) -> Result<Started, Error> {
        self.require_enabled()?;
        let provider = self.providers.enabled_by_slug(tenant, slug).await?;
        // So khớp đúng từng ký tự: không so tiền tố (`https://app.example.com.evil.com`).
        if !provider.redirect_uris().iter().any(|allowed| allowed == redirect_uri) {
            return Err(Error::new(
                autoxide_kernel::error::ErrorKind::Validation,
                "auth.oidc_redirect_not_allowed",
                "The return address `{redirect_uri}` is not in the list of this provider.",
            )
            .with_param("redirect_uri", redirect_uri));
        }
        let authorize = match (provider.kind(), provider.issuer.as_deref()) {
            (Kind::Github, base) => format!("{}/login/oauth/authorize", github_web(base)),
            (_, Some(issuer)) => {
                self.client.discovery(issuer).await?.authorization_endpoint.clone()
            }
            (_, None) => return Err(Error::internal("an OIDC provider has no issuer")),
        };
        let mut url = self.client.check_url(&authorize).map_err(|problem| {
            tracing::warn!(provider_id = %provider.id, why = ?problem, "authorization endpoint refused");
            client::unavailable()
        })?;
        let browser_secret = if bind_browser { Some(random_token(32)?) } else { None };
        let data = StateData {
            nonce: random_token(32)?,
            verifier: random_token(32)?,
            redirect_uri: redirect_uri.to_owned(),
            link_user_id,
            browser: browser_secret.as_deref().map(autoxide_kernel::secret::sha256),
        };
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(data.verifier.as_bytes()));
        let plain = serde_json::to_string(&data).map_err(Error::internal)?;
        let sealer = self.secrets.sealer().await?;
        let row = provider.id.to_string();
        let sealed = sealer.seal(tenant.id(), STATE_PURPOSE, &row, plain.as_bytes())?;
        let ttl = SignedDuration::from_secs(i64::try_from(STATE_TTL_SECONDS).unwrap_or(600));
        let issued = tokens()
            .issue(tenant.database().write(), tenant, PURPOSE, &row, Some(&sealed), ttl)
            .await?;
        let state = link_token(tenant, &issued.token);
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &provider.client_id)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("scope", &provider.scopes)
            .append_pair("state", &state)
            .append_pair("nonce", &data.nonce)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256");
        Ok(Started {
            authorization_url: url.to_string(),
            expires_in: STATE_TTL_SECONDS,
            browser_secret,
        })
    }

    /// Kết thúc lượt đăng nhập: trả khách (lấy từ `state`) và kết quả như `/login`.
    pub(crate) async fn callback(
        &self,
        request: Option<&TenantContext>,
        slug: &str,
        code: &str,
        state: &str,
        browser_secret: Option<&str>,
        user_agent: Option<&str>,
    ) -> Result<(TenantContext, LoginOutcome), Error> {
        self.require_enabled()?;
        let (tenant, provider, data) =
            self.take_state(request, slug, code, state, browser_secret).await?;
        if data.link_user_id.is_some() {
            return Err(state_invalid());
        }
        let external = self.identify(&tenant, &provider, code, &data).await?;
        let user_id = self.resolve_user(&tenant, &provider, &external).await?;
        let outcome = self.sessions.login_verified(&tenant, user_id, "oidc", user_agent).await?;
        Ok((tenant, outcome))
    }

    /// Kết thúc lượt liên kết của người đang đăng nhập `user_id` ở khách `tenant`.
    pub(crate) async fn link_callback(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        slug: &str,
        code: &str,
        state: &str,
        browser_secret: Option<&str>,
    ) -> Result<LinkedIdentity, Error> {
        self.require_enabled()?;
        let (tenant, provider, data) =
            self.take_state(Some(tenant), slug, code, state, browser_secret).await?;
        if data.link_user_id != Some(user_id) {
            return Err(state_invalid());
        }
        let external = self.identify(&tenant, &provider, code, &data).await?;
        self.check_domain(&provider, &external)?;
        let identity = match identity::find(&tenant, provider.id, &external.subject).await? {
            Some(found) if found.user_id == user_id => found,
            Some(_) => return Err(identity::taken()),
            None => self.link(&tenant, user_id, &provider, &external, "user").await?,
        };
        Ok(LinkedIdentity {
            identity,
            provider_slug: provider.slug,
            provider_name: provider.name,
            provider_kind: provider.kind,
        })
    }

    /// Liên kết của người dùng `user_id`, kèm tên nhà cung cấp.
    pub(crate) async fn identities(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
    ) -> Result<Vec<LinkedIdentity>, Error> {
        let providers = self.providers.list(tenant).await?;
        let found = identity::for_user(tenant, user_id).await?;
        Ok(found
            .into_iter()
            .map(|identity| {
                let provider =
                    providers.iter().find(|provider| provider.id == identity.provider_id);
                LinkedIdentity {
                    provider_slug: provider.map(|p| p.slug.clone()).unwrap_or_default(),
                    provider_name: provider.map(|p| p.name.clone()).unwrap_or_default(),
                    provider_kind: provider.map(|p| p.kind.clone()).unwrap_or_default(),
                    identity,
                }
            })
            .collect())
    }

    /// Gỡ liên kết `id` của người dùng `user_id`. Không gỡ cách đăng nhập cuối cùng (không mật
    /// khẩu, không passkey, không liên kết nào khác): `auth.identity_last_method`.
    ///
    /// Kiểm và gỡ trong CÙNG transaction, sau khi khoá dòng người dùng (`rows::lock_one`, D-181):
    /// hai lần gỡ hai liên kết cùng lúc nối đuôi nhau, bên sau đếm lại thấy chỉ còn một cách thì
    /// từ chối. Trước đây kiểm ngoài transaction: trên Postgres cả hai cùng thấy "còn hai liên
    /// kết" và người dùng tự khoá mình ra ngoài (mục 24 của bản rà lỗi đua 11/2026).
    pub(crate) async fn unlink(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        id: Uuid,
    ) -> Result<(), Error> {
        use crate::passkey::entity::passkey;
        use crate::user::entity as user;
        use autoxide_kernel::tenancy::ScopeExt as _;
        use sea_orm::{ColumnTrait as _, EntityTrait as _, PaginatorTrait as _, QueryFilter as _};

        let transaction = tenant.database().begin_write().await?;
        let Some(account) = autoxide_kernel::rows::lock_one(
            &transaction,
            user::Entity::find_by_id(user_id).scoped(tenant),
        )
        .await?
        else {
            transaction.rollback().await?;
            return Err(identity::not_found(id));
        };
        let linked = identity::for_user_in(&transaction, tenant, user_id).await?;
        let Some(target) = linked.iter().find(|identity| identity.id == id) else {
            transaction.rollback().await?;
            return Err(identity::not_found(id));
        };
        let has_passkey = passkey::Entity::find()
            .filter(passkey::Column::UserId.eq(user_id))
            .scoped(tenant)
            .count(&transaction)
            .await?
            > 0;
        if account.password_hash.is_none() && !has_passkey && linked.len() <= 1 {
            transaction.rollback().await?;
            return Err(Error::conflict(
                "auth.identity_last_method",
                "This is your last way to sign in, so it cannot be removed. Set a password or \
                 add a passkey first.",
            ));
        }
        if !identity::delete_one(&transaction, tenant, user_id, id).await? {
            transaction.rollback().await?;
            return Err(identity::not_found(id));
        }
        let event = events::IdentityUnlinked {
            before: Some(json!({"identity_id": id, "provider_id": target.provider_id})),
            after: None,
        };
        let subject = events::user_subject(user_id);
        self.outbox.record(&transaction, tenant, Actor::user(user_id), subject, &event).await?;
        transaction.commit().await?;
        tracing::info!(%user_id, identity_id = %id, "sign-in link removed");
        Ok(())
    }

    /// Dùng `state` một lần; trả khách, nhà cung cấp và dữ liệu của lượt.
    async fn take_state(
        &self,
        request: Option<&TenantContext>,
        slug: &str,
        code: &str,
        state: &str,
        browser_secret: Option<&str>,
    ) -> Result<(TenantContext, Provider, StateData), Error> {
        let problems: Vec<FieldError> = FieldError::blank("code", code)
            .into_iter()
            .chain(FieldError::blank("state", state))
            .collect();
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let Some((tenant, raw)) = crate::token::resolve(&self.databases, request, state).await?
        else {
            return Err(state_invalid());
        };
        let provider = self.providers.enabled_by_slug(&tenant, slug).await?;
        let consumed = tokens().consume(tenant.database().write(), &tenant, PURPOSE, raw).await?;
        let Some(consumed) =
            consumed.filter(|consumed| consumed.subject == provider.id.to_string())
        else {
            return Err(state_invalid());
        };
        let sealed = consumed.data.ok_or_else(state_invalid)?;
        let sealer = self.secrets.sealer().await?;
        let row = provider.id.to_string();
        let plain = sealer.open(tenant.id(), STATE_PURPOSE, &row, &sealed)?;
        let data: StateData = serde_json::from_slice(&plain).map_err(|_| state_invalid())?;
        // Lượt gắn trình duyệt: chỉ nơi giữ `browser_secret` kết thúc được (state đã tiêu: lần
        // thử sai làm hỏng lượt, giao diện mở lượt mới).
        if let Some(expected) = &data.browser
            && browser_secret.map(autoxide_kernel::secret::sha256).as_ref() != Some(expected)
        {
            return Err(state_invalid().with_reason("browser"));
        }
        Ok((tenant, provider, data))
    }

    /// Đổi `code` lấy danh tính đã kiểm.
    async fn identify(
        &self,
        tenant: &TenantContext,
        provider: &Provider,
        code: &str,
        data: &StateData,
    ) -> Result<External, Error> {
        let secret = self.providers.secret(tenant, provider).await?;
        match provider.kind() {
            Kind::Github => self.github(provider, &secret, code, data).await,
            _ => self.openid(provider, &secret, code, data).await,
        }
    }

    async fn openid(
        &self,
        provider: &Provider,
        secret: &str,
        code: &str,
        data: &StateData,
    ) -> Result<External, Error> {
        let issuer = provider
            .issuer
            .as_deref()
            .ok_or_else(|| Error::internal("an OIDC provider has no issuer"))?;
        let discovery = self.client.discovery(issuer).await?;
        let template =
            provider.kind() == Kind::Microsoft && discovery.issuer.contains("{tenantid}");
        if discovery.issuer.trim_end_matches('/') != issuer && !template {
            // OIDC Discovery §4.3: tài liệu phải nói đúng issuer đã hỏi.
            tracing::warn!(
                issuer,
                found = discovery.issuer.as_str(),
                "OIDC discovery names another issuer"
            );
            return Err(client::unavailable());
        }
        let basic = !secret.is_empty()
            && !discovery
                .token_endpoint_auth_methods_supported
                .iter()
                .any(|m| m == "client_secret_post");
        let mut form = vec![
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", data.redirect_uri.as_str()),
            ("code_verifier", data.verifier.as_str()),
        ];
        if !basic {
            form.push(("client_id", provider.client_id.as_str()));
            if !secret.is_empty() {
                form.push(("client_secret", secret));
            }
        }
        let credentials = basic.then_some((provider.client_id.as_str(), secret));
        let answer = self.client.exchange(&discovery.token_endpoint, &form, credentials).await?;
        let raw = answer
            .get("id_token")
            .and_then(Value::as_str)
            .ok_or_else(|| token_invalid("missing_id_token"))?;
        let token = jwt::parse(raw).map_err(reject)?;
        let key = self
            .client
            .key_for(issuer, &discovery.jwks_uri, &token)
            .await?
            .ok_or_else(|| reject(jwt::Rejection::UnknownKey))?;
        token.verify(&key).map_err(reject)?;
        let expected_issuer = if template { discovery.issuer.as_str() } else { issuer };
        let expected = jwt::Expected {
            issuer: expected_issuer,
            alternative_issuer: (provider.kind() == Kind::Google).then_some("accounts.google.com"),
            client_id: &provider.client_id,
            nonce: &data.nonce,
            now: Timestamp::now().as_unix_seconds(),
        };
        let claims = jwt::check_claims(&token.claims, &expected).map_err(reject)?;
        // Entra: claim `email` đổi được và không được xác minh — chỉ tin khi `xms_edov = true`.
        let email_verified = match provider.kind() {
            Kind::Microsoft => claims.email_domain_verified,
            _ => claims.email_verified,
        };
        Ok(External {
            subject: claims.subject,
            email: claims.email.map(|email| autoxide_kernel::email::normalize(&email)),
            email_verified,
            name: claims.name,
        })
    }

    /// GitHub (OAuth 2.0 thuần, không `id_token`): `GET /user` lấy `id` số, `GET /user/emails` lấy
    /// email `primary && verified`.
    async fn github(
        &self,
        provider: &Provider,
        secret: &str,
        code: &str,
        data: &StateData,
    ) -> Result<External, Error> {
        let base = provider.issuer.as_deref();
        let token_url = format!("{}/login/oauth/access_token", github_web(base));
        let form = [
            ("client_id", provider.client_id.as_str()),
            ("client_secret", secret),
            ("code", code),
            ("redirect_uri", data.redirect_uri.as_str()),
            ("code_verifier", data.verifier.as_str()),
        ];
        let answer = self.client.exchange(&token_url, &form, None).await?;
        let Some(access_token) = answer.get("access_token").and_then(Value::as_str) else {
            // GitHub trả 200 kèm `error` khi `code` sai.
            return Err(token_invalid("provider_refused"));
        };
        let api = github_api(base);
        let user = self.client.get_with_token(&format!("{api}/user"), access_token).await?;
        let id =
            user.get("id").and_then(Value::as_u64).ok_or_else(|| token_invalid("malformed"))?;
        let emails =
            self.client.get_with_token(&format!("{api}/user/emails"), access_token).await?;
        let list = emails.as_array().cloned().unwrap_or_default();
        let field =
            |entry: &Value, name: &str| entry.get(name).and_then(Value::as_bool) == Some(true);
        let primary = list.iter().find(|entry| field(entry, "primary"));
        let email = primary.and_then(|entry| entry.get("email")).and_then(Value::as_str);
        let name = user
            .get("name")
            .and_then(Value::as_str)
            .or_else(|| user.get("login").and_then(Value::as_str))
            .map(str::to_owned);
        Ok(External {
            subject: id.to_string(),
            email: email.map(autoxide_kernel::email::normalize),
            email_verified: primary.is_some_and(|entry| field(entry, "verified")),
            name,
        })
    }

    fn check_domain(&self, provider: &Provider, external: &External) -> Result<(), Error> {
        let Some(allowed) = provider.allowed_email_domains() else {
            return Ok(());
        };
        let domain = external
            .email
            .as_deref()
            .filter(|_| external.email_verified)
            .and_then(|email| email.rsplit_once('@'))
            .map(|(_, domain)| domain.to_ascii_lowercase());
        let Some(domain) = domain else {
            return Err(email_unverified());
        };
        if allowed.contains(&domain) {
            Ok(())
        } else {
            Err(Error::forbidden(
                "auth.oidc_email_domain_not_allowed",
                "Email addresses at `{domain}` may not sign in with this provider.",
            )
            .with_param("domain", domain))
        }
    }

    /// Tài khoản cho danh tính `external`: liên kết sẵn có, liên kết mới theo email đã xác minh ở
    /// cả hai phía, hay tài khoản mới khi được phép.
    async fn resolve_user(
        &self,
        tenant: &TenantContext,
        provider: &Provider,
        external: &External,
    ) -> Result<Uuid, Error> {
        self.check_domain(provider, external)?;
        if let Some(found) = identity::find(tenant, provider.id, &external.subject).await? {
            identity::touch(tenant, found.id, external).await?;
            return Ok(found.user_id);
        }
        let realm = provider.realm();
        let existing = match external.email.as_deref() {
            Some(email) => self.users.find_by_email_in(tenant, realm, email).await?,
            None => None,
        };
        match existing {
            Some(user) => {
                // Một trong hai phía chưa xác minh: không gộp (ai đó dựng sẵn tài khoản bằng email
                // của người khác, hay nhà cung cấp cho đặt email tuỳ ý).
                // Nhà cung cấp do người quyền thấp hơn chủ tài khoản cấu hình: không tự gộp
                // (D-20261121-oidc-link-covers) — chủ tài khoản đăng nhập rồi tự liên kết.
                if !external.email_verified
                    || user.email_verified_at.is_none()
                    || !self.configurer_covers(tenant, provider, &user).await?
                {
                    return Err(Error::conflict(
                        "auth.oidc_link_required",
                        "An account with this email already exists. Sign in the usual way, then \
                         link this provider from your account settings.",
                    ));
                }
                self.link(tenant, user.id, provider, external, "verified_email").await?;
                Ok(user.id)
            }
            None => {
                let email = external
                    .email
                    .clone()
                    .filter(|_| external.email_verified)
                    .ok_or_else(email_unverified)?;
                self.signup(tenant, provider, external, realm, email).await
            }
        }
    }

    /// Người cấu hình nhà cung cấp `provider` có mọi permission của tài khoản `user` không. Người
    /// mua không có permission; tài khoản nhân sự không có permission nào thì luôn được.
    async fn configurer_covers(
        &self,
        tenant: &TenantContext,
        provider: &Provider,
        user: &crate::api::User,
    ) -> Result<bool, Error> {
        if user.realm == Realm::Customer {
            return Ok(true);
        }
        let target = self.roles.principal(tenant, user.id).await?;
        if !target.has_all_permissions() && target.permissions().is_empty() {
            return Ok(true);
        }
        let Some(configured_by) = provider.configured_by else { return Ok(false) };
        let Some((by, actor)) = self.roles.user_principal(tenant, configured_by).await? else {
            return Ok(false);
        };
        if !by.is_active() || by.realm != Realm::Staff {
            return Ok(false);
        }
        Ok(if target.has_all_permissions() {
            actor.has_all_permissions()
        } else {
            target.permissions().iter().all(|permission| actor.allows(permission))
        })
    }

    /// Thêm liên kết cho người dùng `user_id` và ghi event.
    async fn link(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        provider: &Provider,
        external: &External,
        reason: &str,
    ) -> Result<Identity, Error> {
        let transaction = tenant.database().begin_write().await?;
        let row = identity::insert(&transaction, tenant, user_id, provider.id, external).await?;
        self.record_link(&transaction, tenant, user_id, provider, &row, reason).await?;
        transaction.commit().await?;
        tracing::info!(%user_id, provider_id = %provider.id, reason, "sign-in linked");
        Ok(row)
    }

    async fn record_link(
        &self,
        transaction: &sea_orm::DatabaseTransaction,
        tenant: &TenantContext,
        user_id: Uuid,
        provider: &Provider,
        row: &Identity,
        reason: &str,
    ) -> Result<(), Error> {
        let event = events::IdentityLinked {
            before: None,
            after: Some(json!({
                "identity_id": row.id,
                "provider_id": provider.id,
                "provider": provider.slug,
                "reason": reason,
            })),
        };
        let subject = events::user_subject(user_id);
        self.outbox.record(transaction, tenant, Actor::user(user_id), subject, &event).await
    }

    /// Tài khoản mới cho danh tính chưa ai có (email đã xác minh).
    async fn signup(
        &self,
        tenant: &TenantContext,
        provider: &Provider,
        external: &External,
        realm: Realm,
        email: String,
    ) -> Result<Uuid, Error> {
        let open = match realm {
            Realm::Customer => {
                crate::settings::enabled(
                    self.orgs.as_ref(),
                    tenant,
                    &crate::settings::CUSTOMER_REGISTRATION,
                )
                .await?
            }
            // Role mặc định nghĩa là quản trị khách chủ động mở cửa cho người của nhà cung cấp
            // này (vd nhân viên trong Entra của công ty) dù đăng ký công khai đóng.
            Realm::Staff => self.self_registration || provider.default_role_id.is_some(),
        };
        if !provider.allow_signup || !open {
            return Err(Error::forbidden(
                "auth.oidc_signup_closed",
                "There is no account for this sign-in, and creating one yourself is not allowed.",
            ));
        }
        let fallback = email.split('@').next().unwrap_or_default().to_owned();
        let name: String = external
            .name
            .clone()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(fallback)
            .chars()
            .take(100)
            .collect();
        let input = NewUser { email: email.clone(), name, password: None };
        let user = match realm {
            Realm::Customer => self.users.create_customer(tenant, Actor::system(), input).await?,
            Realm::Staff => self.users.create(tenant, Actor::system(), input).await?,
        };
        let role = provider.default_role_id;
        let reference = crate::role::member_reference(user.id);
        let limit = &crate::role::MEMBERS_LIMIT;
        // Hạn mức số nhân sự (bước 35): giữ chỗ ngoài transaction, lỗi thì trả lại (D-326).
        if role.is_some() {
            self.entitlements.reserve(tenant.id(), limit, 1, &reference).await?;
        }
        let linked = self.finish_signup(tenant, user.id, provider, external, &email).await;
        if role.is_some() {
            match &linked {
                Ok(()) => self.entitlements.confirm(tenant.id(), limit, &reference).await?,
                Err(_) => self.entitlements.release(tenant.id(), limit, &reference).await?,
            }
        }
        linked?;
        tracing::info!(user_id = %user.id, provider_id = %provider.id, "account created by sign-in provider");
        Ok(user.id)
    }

    async fn finish_signup(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        provider: &Provider,
        external: &External,
        email: &str,
    ) -> Result<(), Error> {
        let now = Timestamp::now();
        let transaction = tenant.database().begin_write().await?;
        // Nhà cung cấp đã xác minh email: tài khoản mới mang email đã xác minh.
        self.users.apply_verified_email(tenant, &transaction, user_id, email, now).await?;
        let row = identity::insert(&transaction, tenant, user_id, provider.id, external).await?;
        if let Some(role) = provider.default_role_id {
            let org = provider
                .default_org_id
                .unwrap_or_else(|| autoxide_kernel::tenancy::root_of(tenant.id()).as_uuid());
            let roles = std::collections::BTreeSet::from([role]);
            crate::role::add_roles(&transaction, tenant, user_id, org, &roles).await?;
        }
        self.record_link(&transaction, tenant, user_id, provider, &row, "signup").await?;
        transaction.commit().await?;
        Ok(())
    }
}

fn github_web(base: Option<&str>) -> String {
    base.map_or_else(|| GITHUB_WEB.to_owned(), |base| base.trim_end_matches('/').to_owned())
}

// GitHub Enterprise Server: API ở `<máy chủ>/api/v3`.
fn github_api(base: Option<&str>) -> String {
    base.map_or_else(
        || GITHUB_API.to_owned(),
        |base| format!("{}/api/v3", base.trim_end_matches('/')),
    )
}

fn state_invalid() -> Error {
    Error::bad_request(
        "auth.oidc_state_invalid",
        "This sign-in attempt has expired or was already used. Start again.",
    )
}

pub(crate) fn token_invalid(reason: &'static str) -> Error {
    Error::unauthenticated(
        "auth.oidc_token_invalid",
        "The sign-in provider sent back sign-in details that are not valid ({reason}). Try again.",
    )
    .with_param("reason", reason)
    .with_reason(reason)
}

fn reject(rejection: jwt::Rejection) -> Error {
    token_invalid(rejection.as_str())
}

fn email_unverified() -> Error {
    Error::forbidden(
        "auth.oidc_email_unverified",
        "The sign-in provider has not verified your email, so it cannot be used to sign in.",
    )
}
