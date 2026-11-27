//! `/token`, `/revoke` và kiểm access token OAuth ở mỗi request (D-20261126-mcp-oauth mục 5, 7).
//!
//! - Đổi mã: mã dùng một lần, sống 60 giây; kiểm PKCE (`S256`), `client_id`, `redirect_uri`, `resource`. Mã dùng lại
//!   (bị đánh cắp và dùng trước) → thu hồi cả grant và mọi token đã sinh từ nó (OAuth 2.1 §4.1.3).
//! - Refresh token xoay vòng mỗi lần dùng. Dùng lại refresh token cũ → thu hồi cả grant, không ân hạn (RFC 9700
//!   §4.14.2): kẻ trộm dùng trước thì người thật gửi lại cũng bị coi là dùng lại; client phải làm mới tuần tự.
//! - Mọi câu "đánh dấu đã dùng" là MỘT câu `UPDATE … WHERE used_at IS NULL`: hai request đồng thời, đúng một thắng
//!   trên cả hai database.

use autoxide_kernel::access::{ActorKind, OAuthAccess, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::secret::random_token;
use autoxide_kernel::tenancy::{ScopeExt as _, TenantContext, TenantId};
use autoxide_kernel::time::Timestamp;
use axum::http::StatusCode;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use jiff::SignedDuration;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait as _, ColumnTrait as _, ConnectionTrait, EntityTrait as _, QueryFilter as _,
    Set,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::consent::CODE_PREFIX;
use super::entity::{grant, token};
use super::{OAuthError, OAuthService, token_digest};

/// Tiền tố của access token và refresh token.
pub(crate) const ACCESS_PREFIX: &str = "axo1_";
pub(crate) const REFRESH_PREFIX: &str = "axr1_";
/// Giá trị cột `kind`.
pub(crate) const CODE: &str = "code";
const ACCESS: &str = "access";
const REFRESH: &str = "refresh";
/// Ghi `last_used_at` của grant thưa nhất mỗi chừng này.
const TOUCH_EVERY: SignedDuration = SignedDuration::from_secs(60);

/// Thân `POST /oauth/token` (`application/x-www-form-urlencoded`, OAuth 2.1 §3.2.2).
#[derive(Debug, Default, Deserialize, ToSchema)]
#[schema(as = OAuthTokenRequest)]
pub(crate) struct TokenForm {
    /// `authorization_code` or `refresh_token`.
    pub grant_type: Option<String>,
    /// The code from the redirect (`authorization_code`).
    pub code: Option<String>,
    /// The PKCE verifier of the request (`authorization_code`).
    pub code_verifier: Option<String>,
    /// The same `redirect_uri` as in the authorization request (`authorization_code`).
    pub redirect_uri: Option<String>,
    /// The refresh token (`refresh_token`).
    pub refresh_token: Option<String>,
    /// The client's `client_id`.
    pub client_id: Option<String>,
    /// The resource, as in the authorization request (RFC 8707). Optional; when sent it must match.
    pub resource: Option<String>,
    /// Ignored when it lists only permissions of the approval; a token never carries more than was approved.
    pub scope: Option<String>,
}

/// Body of `POST /oauth/revoke` (`application/x-www-form-urlencoded`, RFC 7009 §2.1). `token_type_hint` is accepted
/// and ignored.
#[derive(Debug, Default, Deserialize, ToSchema)]
#[schema(as = OAuthRevokeRequest)]
pub(crate) struct RevokeForm {
    /// The access token or refresh token to revoke.
    pub token: Option<String>,
    /// The client's `client_id`.
    pub client_id: Option<String>,
}

/// Successful token response (OAuth 2.1 §3.2.3).
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OAuthTokenResponse)]
pub(crate) struct TokenResponse {
    /// Send it as `Authorization: Bearer <access_token>`.
    pub access_token: String,
    /// Always `Bearer`.
    pub token_type: &'static str,
    /// Seconds until the access token expires.
    pub expires_in: i64,
    /// Use once with `grant_type=refresh_token` to get new tokens; every use returns a new one.
    pub refresh_token: String,
    /// The permissions granted, space-separated.
    pub scope: String,
}

/// Khách ghi trong token; sai dạng → `None`.
fn tenant_of(token: &str, prefix: &str) -> Option<TenantId> {
    OAuthService::tenant_in(token, prefix)
}

impl OAuthService {
    async fn tenant(&self, id: TenantId) -> Result<TenantContext, OAuthError> {
        let tenant = self
            .databases
            .tenant(id)
            .await
            .map_err(|_| OAuthError::invalid_grant("The code or token is not valid."))?;
        autoxide_kernel::tenancy::ensure_serving(&tenant)
            .map_err(|_| OAuthError::invalid_grant("The code or token is not valid."))?;
        Ok(tenant)
    }

    /// `POST /oauth/token`.
    pub(crate) async fn token(&self, form: TokenForm) -> Result<TokenResponse, OAuthError> {
        match form.grant_type.as_deref() {
            Some("authorization_code") => self.exchange_code(form).await,
            Some("refresh_token") => self.refresh(form).await,
            Some(_) => Err(OAuthError::new(
                StatusCode::BAD_REQUEST,
                "unsupported_grant_type",
                "grant_type must be `authorization_code` or `refresh_token`.",
            )),
            None => Err(OAuthError::invalid_request("grant_type is missing.")),
        }
    }

    async fn exchange_code(&self, form: TokenForm) -> Result<TokenResponse, OAuthError> {
        let code =
            form.code.as_deref().ok_or_else(|| OAuthError::invalid_request("code is missing."))?;
        let verifier = form.code_verifier.as_deref().ok_or_else(|| {
            OAuthError::invalid_request("code_verifier is missing (PKCE is required).")
        })?;
        let client_id = form
            .client_id
            .as_deref()
            .ok_or_else(|| OAuthError::invalid_request("client_id is missing."))?;
        let invalid =
            || OAuthError::invalid_grant("The code is not valid, has expired or was already used.");
        let tenant = self.tenant(tenant_of(code, CODE_PREFIX).ok_or_else(invalid)?).await?;
        let secret = code.get(CODE_PREFIX.len() + 33..).unwrap_or_default();
        // Một transaction ngắn trên kết nối ghi: đọc mã, đánh dấu đã dùng (TRƯỚC mọi kiểm khác — mã chỉ đổi được một
        // lần, kể cả khi lần này sai), đọc grant.
        let transaction = tenant.db().begin_write().await?;
        let row = token::Entity::find()
            .filter(token::Column::TokenHash.eq(token_digest(secret)))
            .filter(token::Column::Kind.eq(CODE))
            .scoped(&tenant)
            .one(&transaction)
            .await?
            .ok_or_else(invalid)?;
        let now = Timestamp::now();
        let claimed = mark_used(&transaction, &tenant, row.id, now).await?;
        if !claimed {
            // Mã dùng lại: có thể đã bị đánh cắp. Thu hồi mọi thứ sinh từ nó trong CÙNG transaction với lúc phát hiện:
            // tiến trình chết giữa chừng thì không có trạng thái "đã phát hiện mà grant vẫn sống".
            reuse_detected(&transaction, &tenant, &self.outbox, row.grant_id, "code_reused")
                .await?;
            transaction.commit().await?;
            return Err(invalid());
        }
        let grant = self.live_grant(&transaction, &tenant, row.grant_id).await?;
        transaction.commit().await?;
        if row.expires_at <= now {
            return Err(invalid());
        }
        let grant = grant.ok_or_else(invalid)?;
        if grant.client_id != client_id {
            return Err(invalid());
        }
        if form.redirect_uri.as_deref().is_some_and(|uri| uri != grant.redirect_uri) {
            return Err(OAuthError::invalid_grant(
                "redirect_uri does not match the authorization request.",
            ));
        }
        if let Some(resource) = form.resource.as_deref()
            && self.resource_of(Some(resource)).as_deref() != Some(grant.resource.as_str())
        {
            return Err(OAuthError::new(
                StatusCode::BAD_REQUEST,
                "invalid_target",
                "resource does not match the approval.",
            ));
        }
        if !pkce_matches(verifier, &grant.code_challenge) {
            return Err(OAuthError::invalid_grant(
                "code_verifier does not match the code_challenge.",
            ));
        }
        narrower_scope(form.scope.as_deref(), &grant)?;
        self.issue(&tenant, &grant, now).await
    }

    async fn refresh(&self, form: TokenForm) -> Result<TokenResponse, OAuthError> {
        let presented = form
            .refresh_token
            .as_deref()
            .ok_or_else(|| OAuthError::invalid_request("refresh_token is missing."))?;
        let client_id = form
            .client_id
            .as_deref()
            .ok_or_else(|| OAuthError::invalid_request("client_id is missing."))?;
        let invalid = || {
            OAuthError::invalid_grant("The refresh token is not valid, has expired or was revoked.")
        };
        let tenant = self.tenant(tenant_of(presented, REFRESH_PREFIX).ok_or_else(invalid)?).await?;
        // Đường xoay vòng đọc trên kết nối ghi (transaction ngắn): không đọc bản trễ của read replica.
        let transaction = tenant.db().begin_write().await?;
        let row = token::Entity::find()
            .filter(token::Column::TokenHash.eq(token_digest(presented)))
            .filter(token::Column::Kind.eq(REFRESH))
            .scoped(&tenant)
            .one(&transaction)
            .await?
            .ok_or_else(invalid)?;
        if row.used_at.is_some() {
            // Refresh token đã dùng bị gửi lại: thu hồi ngay, trước mọi kiểm khác (client, scope), cùng transaction.
            metrics::counter!("auth_oauth_refresh_reused_total").increment(1);
            reuse_detected(&transaction, &tenant, &self.outbox, row.grant_id, "refresh_reused")
                .await?;
            transaction.commit().await?;
            return Err(invalid());
        }
        let grant = self.live_grant(&transaction, &tenant, row.grant_id).await?;
        transaction.commit().await?;
        let now = Timestamp::now();
        let grant = grant.ok_or_else(invalid)?;
        if grant.client_id != client_id {
            return Err(invalid());
        }
        if row.expires_at <= now {
            return Err(invalid());
        }
        // Scope sai không được đốt mất refresh token: kiểm trước khi đánh dấu đã dùng.
        narrower_scope(form.scope.as_deref(), &grant)?;
        let transaction = tenant.db().begin_write().await?;
        let claimed = mark_used(&transaction, &tenant, row.id, now).await?;
        if !claimed {
            // Không đánh dấu được: hoặc một request khác vừa dùng token này (dùng lại), hoặc grant vừa bị thu hồi và
            // dòng token đã bị xoá. Chỉ ca thứ nhất là dùng lại.
            let still_there = token::Entity::find_by_id((tenant.id().as_uuid(), row.id))
                .scoped(&tenant)
                .one(&transaction)
                .await?
                .is_some();
            if still_there {
                // Không phân biệt được kẻ trộm với client gửi trùng, nên thu hồi cả grant (RFC 9700 §4.14.2, OAuth
                // 2.1 §4.3.1), cùng transaction với lúc phát hiện. Client phải làm mới tuần tự. Đếm để đo báo nhầm
                // (client làm mới song song) so với đánh cắp thật; grant bị thu hồi giữa chừng không bị đếm.
                metrics::counter!("auth_oauth_refresh_reused_total").increment(1);
                reuse_detected(&transaction, &tenant, &self.outbox, grant.id, "refresh_reused")
                    .await?;
            }
            transaction.commit().await?;
            return Err(invalid());
        }
        transaction.commit().await?;
        self.issue(&tenant, &grant, now).await
    }

    /// Grant còn hiệu lực (chưa thu hồi, chưa hết hạn).
    async fn live_grant<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<Option<grant::Model>, Error> {
        let now = Timestamp::now();
        Ok(grant::Entity::find_by_id((tenant.id().as_uuid(), id))
            .scoped(tenant)
            .one(connection)
            .await?
            .filter(|grant| grant.revoked_at.is_none() && grant.expires_at > now))
    }

    /// Cấp một cặp access + refresh token mới cho `grant`.
    async fn issue(
        &self,
        tenant: &TenantContext,
        grant: &grant::Model,
        now: Timestamp,
    ) -> Result<TokenResponse, OAuthError> {
        // Người dùng phải còn: bị khoá, rời khách hay bị xoá thì không cấp tiếp.
        let alive = self.roles.user_principal(tenant, grant.user_id).await?;
        if !alive.is_some_and(|(user, _)| user.is_active()) {
            return Err(OAuthError::invalid_grant(
                "The account that approved this client can no longer sign in.",
            ));
        }
        let tenant_hex = tenant.id().as_uuid().simple().to_string();
        let access = format!("{ACCESS_PREFIX}{tenant_hex}_{}", random_token(32)?);
        let refresh = format!("{REFRESH_PREFIX}{tenant_hex}_{}", random_token(32)?);
        let access_expires = now + self.config.access_ttl;
        let transaction = tenant.db().begin_write().await?;
        for (kind, value, expires_at) in
            [(ACCESS, &access, access_expires), (REFRESH, &refresh, grant.expires_at)]
        {
            token::ActiveModel {
                tenant_id: Set(tenant.id().as_uuid()),
                id: Set(Uuid::now_v7()),
                grant_id: Set(grant.id),
                kind: Set(kind.to_owned()),
                token_hash: Set(token_digest(value)),
                expires_at: Set(expires_at),
                used_at: Set(None),
                created_at: Set(now),
            }
            .insert(&transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(TokenResponse {
            access_token: access,
            token_type: "Bearer",
            expires_in: self.config.access_ttl.as_secs(),
            refresh_token: refresh,
            scope: grant.scope.clone(),
        })
    }

    /// Bộ xác thực: access token OAuth → người gọi. `invalid` là lỗi 401 chung của token.
    pub(crate) async fn authenticate(
        &self,
        token: &str,
        step_up: &crate::step_up::StepUpPolicy,
    ) -> Result<Principal, Error> {
        let invalid = crate::token::invalid_token;
        let tenant_id = tenant_of(token, ACCESS_PREFIX).ok_or_else(invalid)?;
        let tenant = self.databases.tenant(tenant_id).await.map_err(|_| invalid())?;
        autoxide_kernel::tenancy::ensure_serving(&tenant)?;
        let now = Timestamp::now();
        let row = token::Entity::find()
            .filter(token::Column::TokenHash.eq(token_digest(token)))
            .filter(token::Column::Kind.eq(ACCESS))
            .filter(token::Column::ExpiresAt.gt(now))
            .scoped(&tenant)
            .one(&tenant.db().read())
            .await?
            .ok_or_else(invalid)?;
        let grant = self
            .live_grant(&tenant.db().read(), &tenant, row.grant_id)
            .await?
            .ok_or_else(invalid)?;
        let (user, principal) =
            self.roles.user_principal(&tenant, grant.user_id).await?.ok_or_else(invalid)?;
        if !user.is_active() {
            return Err(crate::user::user_disabled());
        }
        if user.realm == autoxide_kernel::access::Realm::Customer {
            return Err(invalid());
        }
        let principal = principal
            .within_scope(&grant.permissions(), &self.catalog)
            .with_actor_kind(ActorKind::Agent)
            .with_oauth(OAuthAccess::new(
                grant.id,
                grant.client_id.clone(),
                grant.resource.clone(),
            ));
        // Không có lần xác nhận lại nào gắn với token của client: permission nhạy cảm theo chính sách step-up của
        // khách vẫn bị đòi xác nhận (tức là không dùng được qua client).
        let principal = step_up.apply(&tenant, principal, None).await?;
        let principal = self.guardrails.apply(principal).await?;
        if grant.last_used_at.is_none_or(|last| last + TOUCH_EVERY < now) {
            touch(&tenant, grant.id, now).await;
        }
        Ok(principal)
    }

    /// `POST /oauth/revoke` (RFC 7009): thu hồi cả grant của token. Token lạ, sai client: vẫn 200 (§2.2).
    pub(crate) async fn revoke_token(
        &self,
        token: &str,
        client_id: Option<&str>,
    ) -> Result<(), OAuthError> {
        let tenant_id =
            tenant_of(token, ACCESS_PREFIX).or_else(|| tenant_of(token, REFRESH_PREFIX));
        let Some(tenant_id) = tenant_id else { return Ok(()) };
        let Ok(tenant) = self.databases.tenant(tenant_id).await else { return Ok(()) };
        let row = token::Entity::find()
            .filter(token::Column::TokenHash.eq(token_digest(token)))
            .filter(token::Column::Kind.is_in([ACCESS, REFRESH]))
            .scoped(&tenant)
            .one(&tenant.db().read())
            .await?;
        let Some(row) = row else { return Ok(()) };
        let grant = grant::Entity::find_by_id((tenant.id().as_uuid(), row.grant_id))
            .scoped(&tenant)
            .one(&tenant.db().read())
            .await?;
        if grant.is_some_and(|grant| client_id.is_none_or(|client| client == grant.client_id)) {
            super::grants::revoke_grant(self, &tenant, row.grant_id, "client_revoked").await?;
        }
        Ok(())
    }
}

/// `scope` gửi kèm lệnh đổi token chỉ được liệt kê permission đã duyệt (OAuth 2.1 §4.3.1); token luôn mang đúng các
/// permission của grant (không thu hẹp riêng từng token).
fn narrower_scope(scope: Option<&str>, grant: &grant::Model) -> Result<(), OAuthError> {
    let approved = grant.permissions();
    let outside = scope
        .unwrap_or_default()
        .split_whitespace()
        .any(|item| item != "offline_access" && !approved.iter().any(|name| name == item));
    if outside {
        return Err(OAuthError::new(
            StatusCode::BAD_REQUEST,
            "invalid_scope",
            "scope lists a permission the person did not approve.",
        ));
    }
    Ok(())
}

/// `UPDATE … SET used_at = now WHERE id = ? AND used_at IS NULL`: `true` khi câu này là câu đánh dấu.
async fn mark_used<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    id: Uuid,
    now: Timestamp,
) -> Result<bool, Error> {
    let result = token::Entity::update_many()
        .col_expr(token::Column::UsedAt, Expr::value(Some(now)))
        .filter(token::Column::TenantId.eq(tenant.id().as_uuid()))
        .filter(token::Column::Id.eq(id))
        .filter(token::Column::UsedAt.is_null())
        .exec(connection)
        .await?;
    Ok(result.rows_affected == 1)
}

/// Ghi lần dùng gần nhất; hỏng thì chỉ ghi log (không từ chối request hợp lệ vì một dòng thống kê).
async fn touch(tenant: &TenantContext, id: Uuid, now: Timestamp) {
    let result: Result<(), Error> = async {
        let transaction = tenant.db().begin_write().await?;
        grant::Entity::update_many()
            .col_expr(grant::Column::LastUsedAt, Expr::value(Some(now)))
            .filter(grant::Column::TenantId.eq(tenant.id().as_uuid()))
            .filter(grant::Column::Id.eq(id))
            .exec(&transaction)
            .await?;
        transaction.commit().await
    }
    .await;
    if let Err(error) = result {
        tracing::warn!(grant_id = %id, %error, "could not record the use of an OAuth grant");
    }
}

/// PKCE S256 (RFC 7636 §4.6): `BASE64URL(SHA256(verifier)) == challenge`; verifier 43–128 ký tự không dành riêng.
fn pkce_matches(verifier: &str, challenge: &str) -> bool {
    let valid = (43..=128).contains(&verifier.len())
        && verifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~'));
    if !valid {
        return false;
    }
    use sha2::Digest as _;
    let computed = URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier.as_bytes()));
    // So hằng thời gian: độ dài cố định (43), so từng byte không dừng sớm.
    computed.len() == challenge.len()
        && computed
            .bytes()
            .zip(challenge.bytes())
            .fold(0u8, |acc, (left, right)| acc | (left ^ right))
            == 0
}

/// Phát hiện mã hay refresh token bị dùng lại: thu hồi grant `grant_id` trong transaction `transaction` của bên gọi,
/// để phát hiện và thu hồi cùng commit.
async fn reuse_detected<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    outbox: &autoxide_kernel::event::Outbox,
    grant_id: Uuid,
    reason: &'static str,
) -> Result<(), Error> {
    super::grants::revoke_in(transaction, tenant, outbox, Actor::system(), grant_id, reason)
        .await?;
    tracing::info!(grant_id = %grant_id, reason, "OAuth grant revoked");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_s256_theo_rfc_7636() {
        // Verifier sinh ngẫu nhiên mỗi lần chạy; challenge tính theo định nghĩa của RFC 7636 §4.2:
        // BASE64URL(SHA256(ASCII(code_verifier))).
        use sha2::Digest as _;
        let verifier = random_token(32).unwrap();
        let challenge = URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier.as_bytes()));
        assert!(pkce_matches(&verifier, &challenge));
        // Đổi một ký tự của challenge: không khớp.
        let mut other = challenge.clone().into_bytes();
        other[0] = if other[0] == b'A' { b'B' } else { b'A' };
        assert!(!pkce_matches(&verifier, &String::from_utf8(other).unwrap()));
        assert!(!pkce_matches("short", &challenge));
    }

    #[test]
    fn khach_trong_token() {
        let id = Uuid::now_v7();
        let token = format!("{ACCESS_PREFIX}{}_abc", id.simple());
        assert_eq!(tenant_of(&token, ACCESS_PREFIX), Some(TenantId::from_uuid(id)));
        assert_eq!(tenant_of(&token, REFRESH_PREFIX), None);
        assert_eq!(tenant_of("axo1_nothex_abc", ACCESS_PREFIX), None);
        assert_eq!(tenant_of(&format!("{ACCESS_PREFIX}{}_", id.simple()), ACCESS_PREFIX), None);
    }
}
