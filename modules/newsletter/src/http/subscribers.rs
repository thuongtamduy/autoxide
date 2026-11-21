//! Endpoint người đăng ký (`/subscribers…`) và dữ liệu theo email (`/data`). Đọc cần
//! `newsletter.read`; nhập, sửa, xoá cần `newsletter.manage`; xuất / xoá theo email cần
//! `newsletter.export`.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery};
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use super::{choice, page_request, present, required, uuid_param};
use crate::audience::subscribers::{
    ImportInput, ImportPerson, ImportReport, Membership, Subscriber, SubscriberChange,
    SubscriberFilter, SubscriberSort, SubscriberStatus, SubscriptionStatus,
};
use crate::entity::delivery;
use crate::service::NewsletterService;
use crate::text::Locale;

/// A subscription of a subscriber to one list.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct SubscriptionResponse {
    pub list_id: Uuid,
    /// `null` when the list was deleted.
    pub list_key: Option<String>,
    pub status: SubscriptionStatus,
    /// Proof of consent: `{text, version, at, source, ip_fingerprint}` as recorded.
    #[schema(value_type = Option<Object>)]
    pub consent: Option<Value>,
    /// Why the anti-spam checks held the sign-up (it then needed confirming by mail).
    pub hold_reason: Option<String>,
    pub requested_at: Timestamp,
    pub confirmed_at: Option<Timestamp>,
    pub unsubscribed_at: Option<Timestamp>,
}

impl From<Membership> for SubscriptionResponse {
    fn from(membership: Membership) -> Self {
        SubscriptionResponse {
            status: SubscriptionStatus::stored(&membership.row.status),
            list_id: membership.row.list_id,
            list_key: membership.list_key,
            consent: membership.consent,
            hold_reason: membership.row.hold_reason,
            requested_at: membership.row.requested_at,
            confirmed_at: membership.row.confirmed_at,
            unsubscribed_at: membership.row.unsubscribed_at,
        }
    }
}

/// A subscriber (one per email address).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct SubscriberResponse {
    pub id: Uuid,
    /// In lower case.
    #[schema(example = "ada@example.com")]
    pub email: String,
    pub name: Option<String>,
    /// Language of the confirmation mail.
    pub locale: Locale,
    pub status: SubscriberStatus,
    pub blocked_reason: Option<String>,
    /// Set by admins (and imports).
    #[schema(value_type = Object)]
    pub attributes: Map<String, Value>,
    /// Soft bounces since the address was last taken off the suppression list.
    pub soft_bounces: i32,
    /// Why the address is on the suppression list (`hard_bounce`, `soft_bounce`, `complaint`,
    /// `manual`); `null`: it is not.
    pub suppressed: Option<String>,
    pub subscriptions: Vec<SubscriptionResponse>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl From<Subscriber> for SubscriberResponse {
    fn from(subscriber: Subscriber) -> Self {
        SubscriberResponse {
            status: subscriber.status(),
            locale: Locale::stored(&subscriber.row.locale),
            id: subscriber.row.id,
            email: subscriber.row.email,
            name: subscriber.row.name,
            blocked_reason: subscriber.row.blocked_reason,
            attributes: subscriber.attributes,
            soft_bounces: subscriber.row.soft_bounces,
            suppressed: subscriber.suppressed,
            subscriptions: subscriber.subscriptions.into_iter().map(Into::into).collect(),
            created_at: subscriber.row.created_at,
            updated_at: subscriber.row.updated_at,
        }
    }
}

/// Which subscribers to list.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct SubscribersQuery {
    /// Part of the email address, any case.
    pub email: Option<String>,
    /// Only subscribers of this list.
    #[param(value_type = Option<Uuid>)]
    pub list_id: Option<String>,
    /// Only subscriptions in this status (in `list_id`, or in any list).
    #[param(value_type = Option<SubscriptionStatus>, inline)]
    pub status: Option<String>,
    /// `true`: only blocked subscribers (they get no mail); `false`: only the others.
    #[param(value_type = Option<bool>)]
    pub blocked: Option<String>,
    /// `created_at` (the default), `email` or `status`; the direction is `order`. Keep the same
    /// `sort` while following `next_cursor`.
    #[param(value_type = Option<SubscriberSort>, inline)]
    pub sort: Option<String>,
}

/// One person to import.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ImportPersonRequest {
    #[schema(example = "ada@example.com")]
    email: String,
    /// Up to 200 characters.
    name: Option<String>,
    /// Language of their mails; omitted: `en` for a new subscriber.
    locale: Option<Locale>,
    /// Up to 16 KiB of JSON; replaces the attributes of a known subscriber.
    #[schema(value_type = Option<Object>)]
    attributes: Option<Map<String, Value>>,
}

/// People to import into lists, already agreeing to get mail.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ImportRequest {
    /// 1 to 1000 people.
    subscribers: Vec<ImportPersonRequest>,
    /// Lists to put them in (as `confirmed`).
    list_ids: Vec<Uuid>,
    /// Required: where these people agreed to get mail, e.g. `signup-form-2024` (kept as proof).
    #[schema(example = "shop-checkout-2025")]
    consent_source: Option<String>,
    /// The text they agreed to, if known.
    consent_text: Option<String>,
    /// Version of that text, if any.
    consent_version: Option<String>,
}

/// Changes to a subscriber. Omitted fields stay as they are.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateSubscriberRequest {
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    name: Option<Option<String>>,
    locale: Option<Locale>,
    /// `blocked`: no mail at all, whatever the subscriptions.
    status: Option<SubscriberStatus>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    blocked_reason: Option<Option<String>>,
    /// Replaces the attributes; up to 16 KiB of JSON.
    #[schema(value_type = Option<Object>)]
    attributes: Option<Map<String, Value>>,
    /// Unsubscribe from these lists (an admin cannot subscribe someone without consent; use
    /// the import with a consent source).
    #[serde(default)]
    unsubscribe_list_ids: Vec<Uuid>,
}

/// List subscribers.
///
/// Requires `newsletter.read`. Newest first, with their subscriptions; follow
/// `pagination.next_cursor` with the same filters for more.
#[utoipa::path(
    get,
    path = "/subscribers",
    extensions(("x-permissions" = json!(["newsletter.read"]))),
    operation_id = "list_newsletter_subscribers",
    security(("bearer" = [])),
    params(SubscribersQuery, PageQuery),
    responses(
        (status = 200, description = "One page of subscribers.", body = Page<SubscriberResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `newsletter.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_subscribers(
    State(service): State<Arc<NewsletterService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<SubscribersQuery>,
    Query(paging): Query<PageQuery>,
) -> Result<Page<SubscriberResponse>, Error> {
    let mut problems = Vec::new();
    let filter = SubscriberFilter {
        email: query.email,
        list_id: uuid_param(query.list_id.as_deref(), "list_id", &mut problems),
        status: choice(
            query.status.as_deref(),
            "status",
            "`unconfirmed`, `confirmed`, `unsubscribed`",
            &mut problems,
        ),
        blocked: match query.blocked.as_deref().map(str::trim) {
            None | Some("") => None,
            Some("true") => Some(true),
            Some("false") => Some(false),
            Some(_) => {
                problems.push(FieldError::new("blocked", "invalid", "must be `true` or `false`"));
                None
            }
        },
        sort: choice(
            query.sort.as_deref(),
            "sort",
            "`created_at`, `email`, `status`",
            &mut problems,
        )
        .unwrap_or_default(),
    };
    let page = page_request(paging, problems)?;
    Ok(service.list_subscribers(&tenant, &principal, &filter, &page).await?.map(Into::into))
}

/// Import subscribers.
///
/// Requires `newsletter.manage`. Puts up to 1000 people into the lists as `confirmed`, with
/// `consent_source` kept as proof of consent. Known addresses are updated; someone who
/// unsubscribed from a list stays unsubscribed there. Suppressed addresses are imported but
/// get no mail. Every problem is reported at once (paths like `subscribers[3].email`). Retry
/// safely with an `Idempotency-Key` header.
#[utoipa::path(
    post,
    path = "/subscribers/import",
    extensions(("x-permissions" = json!(["newsletter.manage"]))),
    operation_id = "import_newsletter_subscribers",
    security(("bearer" = [])),
    request_body = ImportRequest,
    responses(
        (status = 200, description = "What the import did.", body = Data<ImportReport>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `newsletter.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn import_subscribers(
    State(service): State<Arc<NewsletterService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<ImportRequest>,
) -> Result<Data<ImportReport>, Error> {
    let input = ImportInput {
        people: input
            .subscribers
            .into_iter()
            .map(|person| ImportPerson {
                email: person.email,
                name: person.name,
                locale: person.locale,
                attributes: person.attributes,
            })
            .collect(),
        list_ids: input.list_ids,
        consent_source: input.consent_source,
        consent_text: input.consent_text,
        consent_version: input.consent_version,
    };
    Ok(Data { data: service.import_subscribers(&tenant, &principal, input).await? })
}

/// Get a subscriber.
///
/// Requires `newsletter.read`.
#[utoipa::path(
    get,
    path = "/subscribers/{id}",
    extensions(("x-permissions" = json!(["newsletter.read"]))),
    operation_id = "get_newsletter_subscriber",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the subscriber.")),
    responses(
        (status = 200, description = "The subscriber.", body = Data<SubscriberResponse>, headers(("ETag" = String, description = "Version; send it in `If-Match` to change the subscriber."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `newsletter.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No subscriber has this id; code `newsletter.subscriber_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_subscriber(
    State(service): State<Arc<NewsletterService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Tagged<SubscriberResponse>, Error> {
    let found = service.get_subscriber(&tenant, &principal, id).await?;
    Ok(Tagged { version: found.row.updated_at, data: found.into() })
}

/// Change a subscriber.
///
/// Requires `newsletter.manage`. `If-Match` is required. Block someone with
/// `{"status": "blocked"}`; unsubscribe them from lists with `unsubscribe_list_ids`.
#[utoipa::path(
    patch,
    path = "/subscribers/{id}",
    extensions(("x-permissions" = json!(["newsletter.manage"]))),
    operation_id = "update_newsletter_subscriber",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the subscriber."), ("If-Match" = String, Header, description = "Required: the `ETag` you read (or `*`).")),
    request_body = UpdateSubscriberRequest,
    responses(
        (status = 200, description = "The subscriber after the change.", body = Data<SubscriberResponse>, headers(("ETag" = String, description = "New version."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `newsletter.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No subscriber has this id; code `newsletter.subscriber_not_found`.", body = Problem),
        (status = 412, description = "The subscriber changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn update_subscriber(
    State(service): State<Arc<NewsletterService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<UpdateSubscriberRequest>,
) -> Result<Tagged<SubscriberResponse>, Error> {
    let expected = if_match.required()?;
    let change = SubscriberChange {
        name: input.name,
        locale: input.locale,
        status: input.status,
        blocked_reason: input.blocked_reason,
        attributes: input.attributes,
        unsubscribe_list_ids: input.unsubscribe_list_ids,
    };
    let found = service.update_subscriber(&tenant, &principal, id, change, expected).await?;
    Ok(Tagged { version: found.row.updated_at, data: found.into() })
}

/// Delete a subscriber.
///
/// Requires `newsletter.manage`. Deletes the subscriber for good, with their subscriptions,
/// delivery history and pending confirmation links. A suppression of the address is kept.
#[utoipa::path(
    delete,
    path = "/subscribers/{id}",
    extensions(("x-permissions" = json!(["newsletter.manage"]))),
    operation_id = "delete_newsletter_subscriber",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the subscriber.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `newsletter.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No subscriber has this id; code `newsletter.subscriber_not_found`.", body = Problem),
    )
)]
pub(crate) async fn delete_subscriber(
    State(service): State<Arc<NewsletterService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    service.delete_subscriber(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Whose data.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct EmailQuery {
    /// The email address of the person (any case).
    #[param(example = "ada@example.com")]
    email: Option<String>,
}

/// One mail of a campaign to the person.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = NewsletterDelivery)]
pub(crate) struct DeliveryResponse {
    pub campaign_id: Uuid,
    /// `pending`, `sent`, `failed` or `skipped`.
    pub status: String,
    pub sent_at: Option<Timestamp>,
    /// Why it was skipped or failed.
    pub error: Option<String>,
}

impl From<delivery::Model> for DeliveryResponse {
    fn from(row: delivery::Model) -> Self {
        DeliveryResponse {
            campaign_id: row.campaign_id,
            status: row.status,
            sent_at: row.sent_at,
            error: row.error,
        }
    }
}

/// Everything the newsletter holds about one email address.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = NewsletterEmailData)]
pub(crate) struct EmailDataResponse {
    /// The address, as matched (lower case).
    email: String,
    /// The subscriber with their subscriptions and consent records; `null`: none.
    subscriber: Option<SubscriberResponse>,
    /// Campaign mails to the address.
    deliveries: Vec<DeliveryResponse>,
    /// Why the address is suppressed, and since when; `null`: it is not.
    suppression: Option<super::suppressions::SuppressionResponse>,
}

/// Result of erasing an email address.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = NewsletterErased)]
pub(crate) struct ErasedResponse {
    /// A subscriber was deleted.
    deleted: bool,
}

/// Export the data of an email address.
///
/// Requires `newsletter.export`. For a request of the person the data is about: the
/// subscriber, their subscriptions with consent records, the campaign mails sent to them and
/// a suppression of the address.
#[utoipa::path(
    get,
    path = "/data",
    extensions(("x-permissions" = json!(["newsletter.export"]))),
    operation_id = "export_newsletter_email_data",
    security(("bearer" = [])),
    params(EmailQuery),
    responses(
        (status = 200, description = "The data.", body = Data<EmailDataResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `newsletter.export`; code `permission_denied`.", body = Problem),
        (status = 422, description = "`email` is missing or not an email address; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn get_email_data(
    State(service): State<Arc<NewsletterService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<EmailQuery>,
) -> Result<Data<EmailDataResponse>, Error> {
    // Kiểm quyền trước khi đọc `email`: người không có quyền nhận 403, không phải 422 (D-245).
    principal.require(&crate::permissions::NEWSLETTER_EXPORT)?;
    let email = required(query.email.as_deref(), "email")?;
    let found = service.email_data(&tenant, &principal, email).await?;
    Ok(Data {
        data: EmailDataResponse {
            email: autoxide_kernel::email::normalize(email),
            subscriber: found.subscriber.map(Into::into),
            deliveries: found.deliveries.into_iter().map(Into::into).collect(),
            suppression: found.suppression.map(Into::into),
        },
    })
}

/// Erase the data of an email address.
///
/// Requires `newsletter.export`. Deletes for good the subscriber with this address, their
/// subscriptions, consent records, delivery history and confirmation links (a request of the
/// person the data is about). A suppression of the address is kept, so that it never gets
/// mail again after a bounce or complaint; delete it with `DELETE /suppressions/{id}` if the
/// person asks. The event `newsletter.subscriber_erased` does not carry the address.
#[utoipa::path(
    delete,
    path = "/data",
    extensions(("x-permissions" = json!(["newsletter.export"]))),
    operation_id = "erase_newsletter_email_data",
    security(("bearer" = [])),
    params(EmailQuery),
    responses(
        (status = 200, description = "Whether a subscriber was deleted.", body = Data<ErasedResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `newsletter.export`; code `permission_denied`.", body = Problem),
        (status = 422, description = "`email` is missing or not an email address; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn delete_email_data(
    State(service): State<Arc<NewsletterService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<EmailQuery>,
) -> Result<Data<ErasedResponse>, Error> {
    // Kiểm quyền trước khi đọc `email`: người không có quyền nhận 403, không phải 422 (D-245).
    principal.require(&crate::permissions::NEWSLETTER_EXPORT)?;
    let email = required(query.email.as_deref(), "email")?;
    let deleted = service.erase_email(&tenant, &principal, email).await?;
    Ok(Data { data: ErasedResponse { deleted } })
}
