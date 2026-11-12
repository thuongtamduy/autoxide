//! Endpoint công khai (không cần đăng nhập), dưới `/public`: danh sách công khai, đăng ký, xác
//! nhận, huỷ một chạm, trang lựa chọn. Không bao giờ cho biết một địa chỉ đã có trong danh sách
//! hay chưa.
//!
//! Huỷ một chạm nhận thân `application/x-www-form-urlencoded` (`List-Unsubscribe=One-Click`)
//! như nhà cung cấp hộp thư gửi theo RFC 8058 — ngoại lệ của luật "thân JSON nhận bằng
//! `ValidJson`". Thân không mang dữ liệu gì cần đọc (token nằm ở query), nên endpoint nhận mọi
//! thân và bỏ qua nó.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::{ClientInfo, ValidJson};
use autoxide_kernel::response::{Data, Problem};
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use validator::Validate;

use super::{present, required};
use crate::lists::List;
use crate::public::{Preferences, PreferencesChange, SCOPE, SubscribeRequest};
use crate::service::NewsletterService;
use crate::subscribers::SubscriptionStatus;
use crate::text::{Locale, Localized};

/// A list open to public sign-up, in one language.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct PublicList {
    /// Send it in `lists` to subscribe.
    #[schema(example = "weekly")]
    key: String,
    name: String,
    description: String,
    /// Subscribing needs a click on a link sent by mail.
    double_opt_in: bool,
}

impl PublicList {
    fn new(list: &List, locale: Locale) -> Self {
        PublicList {
            key: list.row.key.clone(),
            name: list.name.get(locale).to_owned(),
            description: list.description.get(locale).to_owned(),
            double_opt_in: list.row.double_opt_in,
        }
    }
}

/// The lists a website may offer.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct PublicListsResponse {
    /// Ask `GET /api/v1/antispam/token?scope=<token_scope>` for a form token each time the
    /// sign-up form is shown, and send it as `form_token`.
    #[schema(example = "newsletter:subscribe")]
    token_scope: String,
    /// Language of the texts.
    locale: Locale,
    lists: Vec<PublicList>,
}

/// Language of the texts.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct LocaleQuery {
    /// `en` (default) or `vi`. A text missing in that language falls back to the other.
    #[param(value_type = Option<Locale>, inline)]
    locale: Option<String>,
}

fn locale(raw: Option<&str>) -> Result<Locale, Error> {
    let mut problems = Vec::new();
    let locale = crate::text::locale_param(raw, "locale", &mut problems);
    if problems.is_empty() {
        Ok(locale.unwrap_or_default())
    } else {
        Err(Error::validation(problems))
    }
}

/// List the lists open to sign-up (public).
///
/// No sign-in needed. The lists marked `public`, by key, with their texts in one language,
/// and the token scope of the sign-up form.
#[utoipa::path(
    get,
    path = "/public/lists",
    operation_id = "list_public_newsletter_lists",
    security((), ("bearer" = [])),
    params(LocaleQuery),
    responses(
        (status = 200, description = "The public lists.", body = Data<PublicListsResponse>),
        (status = 422, description = "`locale` is not `en` or `vi`; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn public_lists(
    State(service): State<Arc<NewsletterService>>,
    _principal: Option<Principal>,
    Query(query): Query<LocaleQuery>,
) -> Result<Data<PublicListsResponse>, Error> {
    let locale = locale(query.locale.as_deref())?;
    let lists = service.public_lists().await?;
    Ok(Data {
        data: PublicListsResponse {
            token_scope: SCOPE.to_owned(),
            locale,
            lists: lists.iter().map(|list| PublicList::new(list, locale)).collect(),
        },
    })
}

/// A sign-up.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SubscribeBody {
    #[schema(example = "ada@example.com")]
    email: Option<String>,
    /// Up to 200 characters. Never put in the confirmation mail.
    name: Option<String>,
    /// Language of the confirmation mail: `en` or `vi`.
    locale: Option<String>,
    /// Keys of public lists, 1 to 20.
    #[serde(default)]
    #[schema(example = json!(["weekly"]))]
    lists: Vec<String>,
    /// The exact consent text shown next to the form (kept as proof), up to 2000 characters.
    consent_text: Option<String>,
    /// Version of that text, if you keep versions.
    consent_version: Option<String>,
    /// Where the form is (kept with the consent), e.g. `footer`; up to 100 characters.
    source: Option<String>,
    /// Token from `GET /api/v1/antispam/token?scope=newsletter:subscribe`; needed unless the
    /// caller is signed in (a website server with an API key).
    form_token: Option<String>,
    /// The hidden trap field: people leave it empty.
    trap: Option<String>,
    /// The value of the `altcha` widget, when the server uses a challenge.
    challenge: Option<String>,
}

/// The answer to a sign-up: always the same.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct SubscribeResponse {
    /// Always `true`. Tell the visitor to check their inbox if a list needs confirming.
    accepted: bool,
}

/// Subscribe to lists (public).
///
/// No sign-in needed. The answer is the same `202` whether the address is new, waiting for
/// confirmation, already subscribed, blocked, or dropped by the anti-spam checks: nobody can
/// learn which addresses are on a list. For lists with double opt-in (and for sign-ups held by
/// the anti-spam checks) a confirmation mail links to `AUTOXIDE_NEWSLETTER_CONFIRM_URL`; at most
/// `AUTOXIDE_NEWSLETTER_CONFIRM_MAILS_PER_DAY` such mails go to one address a day. Sending the
/// same `form_token` again stores nothing new.
#[utoipa::path(
    post,
    path = "/public/subscribe",
    operation_id = "subscribe_newsletter",
    security((), ("bearer" = [])),
    request_body = SubscribeBody,
    responses(
        (status = 202, description = "Received.", body = Data<SubscribeResponse>),
        (status = 400, description = "The form token or challenge is missing, invalid or expired (codes `antispam.token_missing`, `antispam.token_invalid`, `antispam.token_expired`, `antispam.challenge_required`, `antispam.challenge_failed`): reload the form.", body = Problem),
        (status = 422, description = "A value is invalid (e.g. a key that is not a public list); see `errors`.", body = Problem),
        (status = 429, description = "A signed-in caller sent too many; code `antispam.rate_limited`.", body = Problem),
        (status = 503, description = "The server cannot confirm sign-ups (code `newsletter.confirm_url_missing`), or the anti-spam check cannot run (`antispam.verifier_unavailable`).", body = Problem),
    )
)]
pub(crate) async fn subscribe(
    State(service): State<Arc<NewsletterService>>,
    principal: Option<Principal>,
    client: ClientInfo,
    ValidJson(input): ValidJson<SubscribeBody>,
) -> Result<(StatusCode, Data<SubscribeResponse>), Error> {
    let request = SubscribeRequest {
        email: input.email,
        name: input.name,
        locale: input.locale,
        lists: input.lists,
        consent_text: input.consent_text,
        consent_version: input.consent_version,
        source: input.source,
        form_token: input.form_token,
        trap: input.trap,
        challenge: input.challenge,
    };
    service.subscribe(principal.as_ref(), &client, request).await?;
    Ok((StatusCode::ACCEPTED, Data { data: SubscribeResponse { accepted: true } }))
}

/// The token of a confirmation link.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConfirmBody {
    /// The `{token}` of the link in the confirmation mail.
    token: String,
}

/// A list now confirmed.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct ConfirmedList {
    key: String,
    name: Localized,
}

/// The lists confirmed.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct ConfirmResponse {
    lists: Vec<ConfirmedList>,
}

/// Confirm a subscription (public).
///
/// No sign-in needed. The confirmation mail links to the website's page
/// (`AUTOXIDE_NEWSLETTER_CONFIRM_URL`); the page sends the token here with `POST` — opening the
/// link alone (as mail scanners do) changes nothing. A link works once, within
/// `AUTOXIDE_NEWSLETTER_CONFIRM_HOURS`.
#[utoipa::path(
    post,
    path = "/public/confirm",
    operation_id = "confirm_newsletter_subscription",
    security((), ("bearer" = [])),
    request_body = ConfirmBody,
    responses(
        (status = 200, description = "Confirmed.", body = Data<ConfirmResponse>),
        (status = 400, description = "The link is invalid, expired or used; code `newsletter.confirmation_invalid`.", body = Problem),
    )
)]
pub(crate) async fn confirm(
    State(service): State<Arc<NewsletterService>>,
    _principal: Option<Principal>,
    ValidJson(input): ValidJson<ConfirmBody>,
) -> Result<Data<ConfirmResponse>, Error> {
    let lists = service.confirm(&input.token).await?;
    let lists = lists
        .into_iter()
        .map(|list| ConfirmedList { key: list.row.key, name: list.name })
        .collect();
    Ok(Data { data: ConfirmResponse { lists } })
}

/// The token of a mail.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct TokenQuery {
    /// The token in the links of a campaign mail.
    token: Option<String>,
}

/// The answer to an unsubscription: always the same.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct UnsubscribeResponse {
    /// Always `true`.
    unsubscribed: bool,
}

/// Unsubscribe with one click (public).
///
/// No sign-in needed. The `List-Unsubscribe` header of every campaign mail points here; mail
/// apps send `POST` with the form body `List-Unsubscribe=One-Click` (RFC 8058). Unsubscribes
/// the address from the list of that mail at once. The answer is the same for any token.
#[utoipa::path(
    post,
    path = "/public/unsubscribe",
    operation_id = "unsubscribe_newsletter",
    security((), ("bearer" = [])),
    params(TokenQuery),
    request_body(content = String, content_type = "application/x-www-form-urlencoded", description = "`List-Unsubscribe=One-Click` (ignored; may be empty)."),
    responses(
        (status = 200, description = "Unsubscribed.", body = Data<UnsubscribeResponse>),
    )
)]
pub(crate) async fn unsubscribe(
    State(service): State<Arc<NewsletterService>>,
    _principal: Option<Principal>,
    Query(query): Query<TokenQuery>,
    _body: Bytes,
) -> Result<Data<UnsubscribeResponse>, Error> {
    service.unsubscribe(query.token.as_deref().unwrap_or_default()).await?;
    Ok(Data { data: UnsubscribeResponse { unsubscribed: true } })
}

/// One list on the preferences page.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct PreferenceList {
    key: String,
    name: Localized,
    description: Localized,
    /// The person gets this list's campaigns.
    subscribed: bool,
    /// Their subscription, if any.
    status: Option<SubscriptionStatus>,
}

/// What the preferences page shows.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct PreferencesResponse {
    #[schema(example = "ada@example.com")]
    email: String,
    name: Option<String>,
    locale: Locale,
    /// Public lists and the lists the person is on, by key.
    lists: Vec<PreferenceList>,
}

impl From<Preferences> for PreferencesResponse {
    fn from(preferences: Preferences) -> Self {
        PreferencesResponse {
            locale: Locale::stored(&preferences.subscriber.locale),
            email: preferences.subscriber.email,
            name: preferences.subscriber.name,
            lists: preferences
                .lists
                .into_iter()
                .map(|(list, status)| PreferenceList {
                    key: list.row.key,
                    name: list.name,
                    description: list.description,
                    subscribed: status == Some(SubscriptionStatus::Confirmed),
                    status,
                })
                .collect(),
        }
    }
}

/// Get the preferences of a subscriber (public).
///
/// No sign-in needed: the token comes from a campaign mail (the footer links to
/// `AUTOXIDE_NEWSLETTER_PREFERENCES_URL`). Reading changes nothing.
#[utoipa::path(
    get,
    path = "/public/preferences",
    operation_id = "get_newsletter_preferences",
    security((), ("bearer" = [])),
    params(TokenQuery),
    responses(
        (status = 200, description = "The preferences.", body = Data<PreferencesResponse>),
        (status = 404, description = "No subscription matches the token; code `newsletter.link_invalid`.", body = Problem),
        (status = 422, description = "`token` is missing; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn get_preferences(
    State(service): State<Arc<NewsletterService>>,
    _principal: Option<Principal>,
    Query(query): Query<TokenQuery>,
) -> Result<Data<PreferencesResponse>, Error> {
    let token = required(query.token.as_deref(), "token")?;
    Ok(Data { data: service.preferences(token).await?.into() })
}

/// One list choice.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListChoice {
    key: String,
    /// `false` unsubscribes; `true` subscribes at once (public lists only).
    subscribed: bool,
}

/// Changes on the preferences page. Omitted fields stay as they are.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreferencesBody {
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    name: Option<Option<String>>,
    /// `en` or `vi`.
    locale: Option<String>,
    #[serde(default)]
    lists: Vec<ListChoice>,
    /// Unsubscribe from every list.
    #[serde(default)]
    unsubscribe_all: bool,
    /// The consent text shown next to the lists (kept as proof when subscribing).
    consent_text: Option<String>,
    consent_version: Option<String>,
}

/// Change the preferences of a subscriber (public).
///
/// No sign-in needed: the token comes from a campaign mail. Unsubscribing takes effect at
/// once. Subscribing to a public list confirms it at once (the token proves the inbox), with
/// the source `preferences` as proof of consent.
#[utoipa::path(
    put,
    path = "/public/preferences",
    operation_id = "update_newsletter_preferences",
    security((), ("bearer" = [])),
    params(TokenQuery),
    request_body = PreferencesBody,
    responses(
        (status = 200, description = "The preferences after the change.", body = Data<PreferencesResponse>),
        (status = 404, description = "No subscription matches the token; code `newsletter.link_invalid`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn update_preferences(
    State(service): State<Arc<NewsletterService>>,
    _principal: Option<Principal>,
    Query(query): Query<TokenQuery>,
    ValidJson(input): ValidJson<PreferencesBody>,
) -> Result<Data<PreferencesResponse>, Error> {
    let token = required(query.token.as_deref(), "token")?;
    let change = PreferencesChange {
        name: input.name,
        locale: input.locale,
        lists: input.lists.into_iter().map(|choice| (choice.key, choice.subscribed)).collect(),
        unsubscribe_all: input.unsubscribe_all,
        consent_text: input.consent_text,
        consent_version: input.consent_version,
    };
    Ok(Data { data: service.update_preferences(token, change).await?.into() })
}
