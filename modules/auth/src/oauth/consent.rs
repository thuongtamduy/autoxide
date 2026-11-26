//! `/authorize` và API cho trang consent của giao diện (D-20261126-mcp-oauth mục 5, 6, 8).
//!
//! `/authorize` không xác thực: kiểm client và `redirect_uri` trước (sai → 400, KHÔNG chuyển hướng tới địa chỉ chưa
//! tin), rồi các tham số còn lại (sai → chuyển hướng về client kèm `error`, `state`, `iss`). Hợp lệ thì lưu yêu cầu
//! (token một lần, 10 phút, ở khách nền tảng) và chuyển trình duyệt tới trang consent. Trang đó, với phiên đăng nhập
//! của người dùng, đọc yêu cầu rồi duyệt (chọn từng permission) hay từ chối.

use std::collections::BTreeSet;

use autoxide_kernel::access::{Principal, Risk, scope_allows};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::secret::{random_token, sha256};
use autoxide_kernel::tenancy::{ScopeExt as _, TenantContext};
use autoxide_kernel::throttle::{Throttle, address_key};
use autoxide_kernel::time::Timestamp;
use autoxide_kernel::tokens::OneTimeTokens;
use jiff::SignedDuration;
use reqwest::Url;
use sea_orm::{
    ActiveModelTrait as _, ColumnTrait as _, EntityTrait as _, QueryFilter as _, QueryOrder as _,
    Set,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::clients::{host_of, localhost_only};
use super::entity::{grant, token};
use super::{OAuthService, require_person};
use crate::events;

const PURPOSE: &str = "oauth_request";
const REQUEST_MINUTES: i64 = 10;
const CODE_SECONDS: i64 = 60;
/// Tiền tố của mã uỷ quyền.
pub(crate) const CODE_PREFIX: &str = "axc1_";
/// Yêu cầu `/authorize` mỗi địa chỉ mỗi phút: endpoint chưa xác thực, có ghi database và có thể tải tài liệu ngoài.
const AUTHORIZE_PER_MINUTE: u32 = 30;
/// Độ dài tối đa của `client_id` và `redirect_uri` (cột lưu chúng là 2048 ký tự).
pub(crate) const MAX_URI: usize = 2048;

/// Query của `/authorize` (OAuth 2.1 §4.1.1, RFC 8707, MCP 2026-07-28).
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct AuthorizeQuery {
    /// Must be `code`.
    pub response_type: Option<String>,
    /// URL of the client's metadata document, or the `client_id` from dynamic registration.
    pub client_id: Option<String>,
    /// Where the browser returns; must match one the client registered.
    pub redirect_uri: Option<String>,
    /// PKCE code challenge (base64url SHA-256 of the verifier, 43 characters).
    pub code_challenge: Option<String>,
    /// Must be `S256`.
    pub code_challenge_method: Option<String>,
    /// Opaque value returned unchanged to the client.
    pub state: Option<String>,
    /// Space-separated permissions (`notes.read`) or prefixes (`notes.*`) the client asks for. Omit to let the
    /// person choose.
    pub scope: Option<String>,
    /// The resource the token is for: `{issuer}/mcp` (default), `{issuer}/mcp/{module}` or `{issuer}/api/v1`.
    pub resource: Option<String>,
}

/// Yêu cầu đang chờ (dữ liệu của token một lần).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Pending {
    client_id: String,
    client_name: String,
    registration: String,
    redirect_uri: String,
    code_challenge: String,
    resource: String,
    scope: Vec<String>,
    state: Option<String>,
    expires_at: Timestamp,
}

/// One permission the person can grant.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = OAuthOfferedPermission)]
pub(crate) struct OfferedPermission {
    /// Permission name, e.g. `notes.write`.
    pub name: String,
    /// What it allows, for the person deciding.
    pub description: String,
    /// `read`, `write`, `destructive`, `sensitive_data` or `money`. The last three need a recent re-confirmation
    /// (`reauth_token`) to grant.
    pub risk: String,
    /// The client asked for it (directly or through a prefix).
    pub requested: bool,
}

/// An authorization request waiting for the signed-in person's decision.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = OAuthRequest)]
pub(crate) struct RequestView {
    /// The client's identifier: for a metadata-document client, the URL of the document.
    pub client_id: String,
    /// Host of `client_id`; show it next to the name, which the client chose itself and is not verified.
    pub client_host: String,
    /// Name the client gave itself (unverified).
    pub client_name: String,
    /// `cimd` (client metadata document) or `dynamic` (self-registered).
    pub registration: String,
    /// Where the browser goes back to.
    pub redirect_uri: String,
    /// Host of `redirect_uri`; always show it.
    pub redirect_host: String,
    /// The redirect points to this device (`localhost`, a loopback address): another program on the device could
    /// pretend to be this client. Warn the person.
    pub localhost_redirect: bool,
    /// The resource the client will use the access for.
    pub resource: String,
    /// What the client asked for (empty: the person chooses).
    pub requested_scope: Vec<String>,
    /// Permissions the person can grant (those they hold, narrowed to what the client asked for).
    pub permissions: Vec<OfferedPermission>,
    /// Permissions of the person's current approval of this client for this resource, if any.
    pub current_permissions: Option<Vec<String>>,
    /// Until when the request waits.
    pub expires_at: Timestamp,
}

/// The person's approval.
#[derive(Debug, Deserialize, ToSchema, validator::Validate)]
#[serde(deny_unknown_fields)]
#[schema(as = OAuthDecision)]
pub(crate) struct Decision {
    /// The permissions to grant, chosen one by one from `permissions` of the request.
    pub permissions: Vec<String>,
    /// How many days the approval lasts (1 to the server's maximum, default the maximum).
    pub days: Option<i64>,
    /// A token from `POST /api/v1/auth/me/reauth`: required when a chosen permission is `destructive`,
    /// `sensitive_data` or `money`.
    pub reauth_token: Option<String>,
}

/// Where to send the browser after a decision.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = OAuthDecisionResult)]
pub(crate) struct DecisionResult {
    /// Open this address in the browser: the client's `redirect_uri` with `code` (or `error`), `state` and `iss`.
    pub redirect_to: String,
}

fn request_gone() -> Error {
    Error::not_found(
        "auth.oauth_request_not_found",
        "This authorization request does not exist or has expired (requests wait 10 minutes). Start \
         connecting the client again.",
    )
}

fn client_invalid(detail: String) -> Error {
    Error::bad_request("auth.oauth_client_invalid", "The client is not valid: {detail}")
        .with_param("detail", detail)
}

fn redirect_invalid(detail: String) -> Error {
    Error::bad_request(
        "auth.oauth_redirect_invalid",
        "The redirect address is not one the client registered: {detail}",
    )
    .with_param("detail", detail)
}

/// Rủi ro cần xác nhận lại khi cấp (D-20261126-mcp-oauth mục 8).
fn sensitive(risk: Risk) -> bool {
    matches!(risk, Risk::Destructive | Risk::Money | Risk::SensitiveData)
}

impl OAuthService {
    /// `/authorize`. `client`: địa chỉ người gọi (hạn mức).
    pub(crate) async fn authorize(
        &self,
        query: AuthorizeQuery,
        client: Option<std::net::IpAddr>,
    ) -> Result<String, Error> {
        let platform = self.platform().await?;
        if let Some(ip) = client {
            let throttle =
                Throttle::new("auth", AUTHORIZE_PER_MINUTE, std::time::Duration::from_secs(60))?;
            if !throttle.hit(&platform, &address_key("oauth_authorize", ip)).await?.is_allowed() {
                return Err(Error::too_many_requests(
                    "auth.oauth_rate_limited",
                    "Too many authorization requests from this address; wait a minute and retry.",
                )
                .with_retry_after(60));
            }
        }
        // `client_id` và `redirect_uri` tối đa 2048 ký tự (cột lưu chúng): kiểm trước khi đọc hay lưu.
        let too_long =
            |value: &Option<String>| value.as_ref().is_some_and(|value| value.len() > MAX_URI);
        if too_long(&query.client_id) {
            return Err(client_invalid("client_id is longer than 2048 characters".to_owned()));
        }
        if too_long(&query.redirect_uri) {
            return Err(redirect_invalid("redirect_uri is longer than 2048 characters".to_owned()));
        }
        let client_id = query
            .client_id
            .as_deref()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| client_invalid("client_id is missing".to_owned()))?;
        let registered = self
            .clients
            .resolve(&platform, client_id)
            .await
            .map_err(|error| client_invalid(error.description))?;
        let redirect_uri = match query.redirect_uri.as_deref() {
            Some(uri) if registered.allows_redirect(uri) => uri.to_owned(),
            None if registered.redirect_uris.len() == 1 => registered.redirect_uris[0].clone(),
            _ => {
                return Err(redirect_invalid(query.redirect_uri.unwrap_or_default()));
            }
        };
        let state = query.state.clone();
        // Client tự đăng ký (động hay tài liệu metadata) chọn `redirect_uri` của chính nó: chuyển hướng lỗi về đó biến
        // máy chủ thành bộ chuyển hướng mở (RFC 9700 §4.11.2). Mọi lỗi tham số trả 400 ở đây.
        let fail = |error: &str, description: &str| -> Result<String, Error> {
            Err(Error::bad_request(
                "auth.oauth_request_invalid",
                "The authorization request is not valid (`{error}`): {detail}",
            )
            .with_param("error", error)
            .with_param("detail", description))
        };
        if query.response_type.as_deref() != Some("code") {
            return fail("unsupported_response_type", "response_type must be `code`.");
        }
        let challenge = query.code_challenge.unwrap_or_default();
        if query.code_challenge_method.as_deref() != Some("S256") || !is_s256_challenge(&challenge)
        {
            return fail(
                "invalid_request",
                "PKCE is required: send code_challenge with code_challenge_method=S256.",
            );
        }
        let Some(resource) = self.resource_of(query.resource.as_deref()) else {
            return fail(
                "invalid_target",
                "resource must be this server's MCP endpoint or REST API.",
            );
        };
        let Some(scope) = self.parse_scope(query.scope.as_deref()) else {
            return fail(
                "invalid_scope",
                "scope lists a permission or prefix this server does not have.",
            );
        };
        let pending = Pending {
            client_id: registered.client_id,
            client_name: registered.name,
            registration: registered.registration.to_owned(),
            redirect_uri,
            code_challenge: challenge,
            resource,
            scope,
            state,
            expires_at: Timestamp::now() + SignedDuration::from_mins(REQUEST_MINUTES),
        };
        let data = serde_json::to_string(&pending).map_err(Error::internal)?;
        let issued = OneTimeTokens::new("auth")
            .issue(
                platform.database().write(),
                &platform,
                PURPOSE,
                "oauth",
                Some(&data),
                SignedDuration::from_mins(REQUEST_MINUTES),
            )
            .await?;
        Ok(self.config.consent_url.replace("{token}", &issued.token))
    }

    /// `scope` → danh sách token hợp lệ (`offline_access` bỏ qua); `None` khi có token lạ.
    fn parse_scope(&self, raw: Option<&str>) -> Option<Vec<String>> {
        let mut scope = Vec::new();
        for item in raw.unwrap_or_default().split_whitespace() {
            if item == "offline_access" {
                continue;
            }
            let known = match item.strip_suffix(".*") {
                Some(_) => {
                    self.catalog.iter().any(|permission| scope_allows([item], permission.name()))
                }
                None => self.catalog.contains(item),
            };
            if !known {
                return None;
            }
            if !scope.iter().any(|existing| existing == item) {
                scope.push(item.to_owned());
            }
        }
        Some(scope)
    }

    /// `redirect_uri` kèm tham số và `state`, `iss` (RFC 9207: mọi phản hồi, cả lỗi).
    fn answer(&self, redirect_uri: &str, pairs: &[(&str, &str)], state: Option<&str>) -> String {
        let Ok(mut url) = Url::parse(redirect_uri) else { return redirect_uri.to_owned() };
        {
            let mut query = url.query_pairs_mut();
            for (name, value) in pairs {
                query.append_pair(name, value);
            }
            if let Some(state) = state {
                query.append_pair("state", state);
            }
            query.append_pair("iss", &self.issuer);
        }
        url.to_string()
    }

    async fn pending(&self, token: &str) -> Result<(Pending, Timestamp), Error> {
        let platform = self.platform().await?;
        let tokens = OneTimeTokens::new("auth");
        let found =
            tokens.peek(platform.database().read(), &platform, PURPOSE, token.trim()).await?;
        let found = found.ok_or_else(request_gone)?;
        let pending: Pending = serde_json::from_str(found.data.as_deref().unwrap_or_default())
            .map_err(|_| request_gone())?;
        let expires_at = pending.expires_at;
        Ok((pending, expires_at))
    }

    /// Permission người duyệt cấp được: quyền hiện có, thu theo `scope` client xin (rỗng: mọi quyền).
    fn offered(&self, principal: &Principal, pending: &Pending) -> Vec<OfferedPermission> {
        let held: Vec<String> = match principal.grant_map() {
            None => self.catalog.iter().map(|permission| permission.name().to_owned()).collect(),
            Some(map) => map.keys().cloned().collect(),
        };
        let asked = |name: &str| scope_allows(pending.scope.iter().map(String::as_str), name);
        self.catalog
            .iter()
            .filter(|permission| held.iter().any(|name| name == permission.name()))
            .filter(|permission| pending.scope.is_empty() || asked(permission.name()))
            .map(|permission| OfferedPermission {
                name: permission.name().to_owned(),
                description: permission.description().to_owned(),
                risk: permission.risk().as_str().to_owned(),
                requested: asked(permission.name()),
            })
            .collect()
    }

    /// `GET /oauth/requests/{token}`.
    pub(crate) async fn describe(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        token: &str,
    ) -> Result<RequestView, Error> {
        require_person(principal)?;
        let (pending, expires_at) = self.pending(token).await?;
        let current = grant::Entity::find()
            .filter(grant::Column::UserId.eq(principal.user_id()))
            .filter(grant::Column::ClientId.eq(pending.client_id.as_str()))
            .filter(grant::Column::Resource.eq(pending.resource.as_str()))
            .filter(grant::Column::RevokedAt.is_null())
            .filter(grant::Column::ExpiresAt.gt(Timestamp::now()))
            .scoped(tenant)
            .order_by_desc(grant::Column::Id)
            .one(tenant.database().read())
            .await?;
        Ok(RequestView {
            client_host: host_of(&pending.client_id),
            client_name: pending.client_name.clone(),
            registration: pending.registration.clone(),
            redirect_host: host_of(&pending.redirect_uri),
            localhost_redirect: localhost_only(std::slice::from_ref(&pending.redirect_uri)),
            redirect_uri: pending.redirect_uri.clone(),
            resource: pending.resource.clone(),
            requested_scope: pending.scope.clone(),
            permissions: self.offered(principal, &pending),
            current_permissions: current.map(|grant| grant.permissions()),
            client_id: pending.client_id,
            expires_at,
        })
    }

    /// `POST /oauth/requests/{token}/approve`: tạo grant và mã uỷ quyền.
    pub(crate) async fn approve(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        token: &str,
        decision: Decision,
    ) -> Result<String, Error> {
        require_person(principal)?;
        let (pending, _) = self.pending(token).await?;
        let offered = self.offered(principal, &pending);
        let mut problems = Vec::new();
        let chosen: BTreeSet<String> = decision.permissions.iter().cloned().collect();
        if chosen.is_empty() {
            problems.push(FieldError::new(
                "permissions",
                "blank",
                "choose at least one permission",
            ));
        }
        let mut needs_reauth = false;
        for name in &chosen {
            match offered.iter().find(|offer| &offer.name == name) {
                Some(offer) => {
                    let risk = self
                        .catalog
                        .iter()
                        .find(|permission| permission.name() == name)
                        .map(|permission| permission.risk());
                    needs_reauth |= risk.is_some_and(sensitive);
                    let _ = offer;
                }
                None => problems.push(
                    FieldError::new(
                        "permissions",
                        "not_offered",
                        "`{name}` is not one of the permissions you can grant to this request",
                    )
                    .with_param("name", name),
                ),
            }
        }
        let days = decision.days.unwrap_or(self.config.grant_days);
        if !(1..=self.config.grant_days).contains(&days) {
            problems.push(
                FieldError::new("days", "out_of_range", "must be from 1 to {max}")
                    .with_param("max", self.config.grant_days),
            );
        }
        if needs_reauth
            && decision.reauth_token.as_deref().is_none_or(|value| value.trim().is_empty())
        {
            problems.push(FieldError::new(
                "reauth_token",
                "required",
                "a chosen permission can delete data, touch money or read personal data: confirm it is you with \
                 `POST /api/v1/auth/me/reauth` and send the token it returns",
            ));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        if needs_reauth && let Some(reauth) = &decision.reauth_token {
            crate::reauth::consume(tenant, principal.user_id(), reauth).await?;
        }
        // Yêu cầu chỉ dùng một lần: hai lần bấm duyệt đồng thời, đúng một lần thắng.
        let platform = self.platform().await?;
        OneTimeTokens::new("auth")
            .consume(platform.database().write(), &platform, PURPOSE, token.trim())
            .await?
            .ok_or_else(request_gone)?;
        let now = Timestamp::now();
        let grant_id = Uuid::now_v7();
        let code = random_token(32)?;
        let scope = chosen.into_iter().collect::<Vec<_>>().join(" ");
        let transaction = tenant.database().begin_write().await?;
        grant::ActiveModel {
            tenant_id: Set(tenant.id().as_uuid()),
            id: Set(grant_id),
            user_id: Set(principal.user_id()),
            client_id: Set(pending.client_id.clone()),
            client_name: Set(pending.client_name.clone()),
            registration: Set(pending.registration.clone()),
            redirect_uri: Set(pending.redirect_uri.clone()),
            resource: Set(pending.resource.clone()),
            code_challenge: Set(pending.code_challenge.clone()),
            scope: Set(scope.clone()),
            created_at: Set(now),
            expires_at: Set(now + SignedDuration::from_hours(24 * days)),
            last_used_at: Set(None),
            revoked_at: Set(None),
        }
        .insert(&transaction)
        .await?;
        token::ActiveModel {
            tenant_id: Set(tenant.id().as_uuid()),
            id: Set(Uuid::now_v7()),
            grant_id: Set(grant_id),
            kind: Set(super::tokens::CODE.to_owned()),
            token_hash: Set(sha256(&code)),
            expires_at: Set(now + SignedDuration::from_secs(CODE_SECONDS)),
            used_at: Set(None),
            created_at: Set(now),
        }
        .insert(&transaction)
        .await?;
        let event = events::OauthGranted {
            before: None,
            after: Some(serde_json::json!({
                "grant_id": grant_id,
                "user_id": principal.user_id(),
                "client_id": pending.client_id,
                "client_name": pending.client_name,
                "resource": pending.resource,
                "permissions": scope.split(' ').collect::<Vec<_>>(),
                "days": days,
            })),
        };
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                super::grants::subject(grant_id),
                &event,
            )
            .await?;
        transaction.commit().await?;
        tracing::info!(%grant_id, client_id = pending.client_id, "OAuth client approved");
        let code = format!("{CODE_PREFIX}{}_{code}", tenant.id().as_uuid().simple());
        Ok(self.answer(&pending.redirect_uri, &[("code", &code)], pending.state.as_deref()))
    }

    /// `POST /oauth/requests/{token}/deny`.
    pub(crate) async fn deny(&self, principal: &Principal, token: &str) -> Result<String, Error> {
        require_person(principal)?;
        let (pending, _) = self.pending(token).await?;
        let platform = self.platform().await?;
        OneTimeTokens::new("auth")
            .consume(platform.database().write(), &platform, PURPOSE, token.trim())
            .await?
            .ok_or_else(request_gone)?;
        Ok(self.answer(
            &pending.redirect_uri,
            &[
                ("error", "access_denied"),
                ("error_description", "The person declined the request."),
            ],
            pending.state.as_deref(),
        ))
    }
}

/// `code_challenge` của S256: 43 ký tự base64url (SHA-256, không đệm).
fn is_s256_challenge(challenge: &str) -> bool {
    challenge.len() == 43
        && challenge
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}
