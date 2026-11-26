//! Route REST cho quản trị (dưới `/api/v1/notifications`): câu chữ thay thế, gửi thông báo, nhật ký
//! gửi, thông báo của nền tảng. Permission được kiểm trong service TRƯỚC mọi bước tra cứu hay kiểm
//! đầu vào.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::api::{Notification, Outcome, Recipient};
use crate::entity::{message, template};
use crate::messages::MessageFilter;
use crate::permissions::SEND;
use crate::platform::NewPlatformNotice;
use crate::service::NotificationsService;
use crate::templates::TemplateInput;

pub(crate) fn routes(service: Arc<NotificationsService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_templates))
        .routes(routes!(put_template, delete_template))
        .routes(routes!(send_notification, list_messages))
        .routes(routes!(get_message))
        .routes(routes!(send_platform_notice))
        .with_state(service)
}

// ---- Mẫu -----------------------------------------------------------------------------------------

/// A text of the tenant that replaces the default text of a kind on one channel and language.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = NotificationTemplate)]
pub(crate) struct TemplateResponse {
    kind: String,
    channel: String,
    locale: String,
    title: String,
    body: String,
    /// For SMS, ZNS and push.
    short: String,
    /// Approved Zalo ZNS template id (channel `zns`).
    zns_template_id: Option<String>,
    updated_at: Timestamp,
}

impl From<template::Model> for TemplateResponse {
    fn from(row: template::Model) -> Self {
        TemplateResponse {
            kind: row.kind,
            channel: row.channel,
            locale: row.locale,
            title: row.title,
            body: row.body,
            short: row.short,
            zns_template_id: row.zns_template_id,
            updated_at: row.updated_at,
        }
    }
}

/// A text to replace the default one. Use only the `{name}` values of the default text (and
/// `{link}`).
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = NotificationTemplateRequest)]
pub(crate) struct TemplateRequest {
    #[schema(example = "Order {code} is on its way")]
    title: String,
    body: String,
    short: String,
    zns_template_id: Option<String>,
}

/// List the replaced texts of the tenant.
///
/// Requires `notifications.templates.manage`. Kinds and channels without a row use the default text.
#[utoipa::path(
    get,
    path = "/templates",
    extensions(("x-permissions" = json!(["notifications.templates.manage"]))),
    operation_id = "list_notification_templates",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Every replaced text.", body = Data<Vec<TemplateResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `notifications.templates.manage`; code `permission_denied`.", body = Problem),
    )
)]
async fn list_templates(
    State(service): State<Arc<NotificationsService>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<Vec<TemplateResponse>>, Error> {
    let rows = service.list_templates(&tenant, &principal).await?;
    Ok(Data { data: rows.into_iter().map(TemplateResponse::from).collect() })
}

/// Replace the text of a kind on one channel and language.
///
/// Requires `notifications.templates.manage`. Advertising always gets the label and the
/// unsubscribe link added; a text cannot remove them.
#[utoipa::path(
    put,
    path = "/templates/{kind}/{channel}/{locale}",
    extensions(("x-permissions" = json!(["notifications.templates.manage"]))),
    operation_id = "set_notification_template",
    security(("bearer" = [])),
    params(
        ("kind" = String, Path, description = "Key of the kind, e.g. `orders.order_shipped`."),
        ("channel" = String, Path, description = "One channel of the kind, e.g. `email`."),
        ("locale" = String, Path, description = "`en` or `vi`."),
    ),
    request_body = TemplateRequest,
    responses(
        (status = 200, description = "The replaced text.", body = Data<TemplateResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `notifications.templates.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "Unknown kind (`notifications.kind_unknown`), a value the kind does not have (`notifications.template_invalid`), or another invalid value (every problem in `errors`).", body = Problem),
    )
)]
async fn put_template(
    State(service): State<Arc<NotificationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((kind, channel, locale)): Path<(String, String, String)>,
    ValidJson(input): ValidJson<TemplateRequest>,
) -> Result<Data<TemplateResponse>, Error> {
    let input = TemplateInput {
        title: input.title,
        body: input.body,
        short: input.short,
        zns_template_id: input.zns_template_id,
    };
    let row = service.put_template(&tenant, &principal, (&kind, &channel, &locale), input).await?;
    Ok(Data { data: row.into() })
}

/// Go back to the default text of a kind on one channel and language.
///
/// Requires `notifications.templates.manage`.
#[utoipa::path(
    delete,
    path = "/templates/{kind}/{channel}/{locale}",
    extensions(("x-permissions" = json!(["notifications.templates.manage"]))),
    operation_id = "delete_notification_template",
    security(("bearer" = [])),
    params(
        ("kind" = String, Path, description = "Key of the kind."),
        ("channel" = String, Path, description = "Channel."),
        ("locale" = String, Path, description = "`en` or `vi`."),
    ),
    responses(
        (status = 204, description = "Removed; the default text applies."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `notifications.templates.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No replaced text there; code `notifications.template_not_found`.", body = Problem),
        (status = 422, description = "Unknown kind, channel or language; code `notifications.kind_unknown` or `validation_failed`.", body = Problem),
    )
)]
async fn delete_template(
    State(service): State<Arc<NotificationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((kind, channel, locale)): Path<(String, String, String)>,
) -> Result<StatusCode, Error> {
    service.delete_template(&tenant, &principal, (&kind, &channel, &locale)).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- Gửi và nhật ký ------------------------------------------------------------------------------

/// Who gets it: exactly one of the fields.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = NotificationRecipient)]
pub(crate) struct RecipientRequest {
    /// A user of the tenant (email from the account, in-app, push).
    pub user_id: Option<Uuid>,
    /// An email address.
    pub email: Option<String>,
    /// A phone number, e.g. `+84912345678`.
    pub phone: Option<String>,
    /// `true`: every active admin of the tenant.
    #[serde(default)]
    pub tenant_admins: bool,
}

impl RecipientRequest {
    pub(crate) fn into_recipient(self) -> Result<Recipient, Error> {
        let mut found = Vec::new();
        if let Some(user) = self.user_id {
            found.push(Recipient::User(user));
        }
        if let Some(email) = self.email {
            found.push(Recipient::Email(email));
        }
        if let Some(phone) = self.phone {
            found.push(Recipient::Phone(phone));
        }
        if self.tenant_admins {
            found.push(Recipient::TenantAdmins);
        }
        match found.len() {
            1 => found.pop().ok_or_else(|| Error::internal("a recipient vanished")),
            _ => Err(Error::validation(vec![FieldError::new(
                "recipient",
                "invalid",
                "give exactly one of `user_id`, `email`, `phone` or `tenant_admins`",
            )])),
        }
    }
}

/// A notification to send.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = NotificationRequest)]
pub(crate) struct SendRequest {
    /// Key of a declared kind (`GET /kinds`).
    #[schema(example = "notifications.platform_notice")]
    kind: String,
    recipient: RecipientRequest,
    /// Values for the `{name}` places of the text (display text only).
    #[serde(default)]
    #[schema(value_type = Object)]
    params: Value,
    /// Page opened from the notification: an http(s) URL or a path.
    link: Option<String>,
    /// `en` or `vi`; default the language of the user, else English.
    locale: Option<String>,
    /// Required: sending again with the same key sends nothing new.
    #[schema(example = "order-123-shipped")]
    dedupe_key: String,
}

/// Send a notification.
///
/// Requires `notifications.send`; advertising also `notifications.send_marketing`. Channels without
/// consent, preference, address or provider are skipped and listed in `skipped`. Delivery runs in
/// the background; follow it with `GET /messages/{id}`.
#[utoipa::path(
    post,
    path = "/messages",
    extensions(("x-permissions" = json!(["notifications.send"]))),
    operation_id = "send_notification",
    security(("bearer" = [])),
    request_body = SendRequest,
    responses(
        (status = 200, description = "What was queued and what was skipped.", body = Data<Outcome>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `notifications.send` (or `notifications.send_marketing` for advertising); code `permission_denied`.", body = Problem),
        (status = 422, description = "Unknown kind (`notifications.kind_unknown`), unknown user (`notifications.recipient_unknown`), or an invalid value (every problem in `errors`).", body = Problem),
    )
)]
async fn send_notification(
    State(service): State<Arc<NotificationsService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<SendRequest>,
) -> Result<Data<Outcome>, Error> {
    // Kiểm permission trước cả khi đọc người nhận.
    principal.require(&SEND)?;
    let notification = Notification {
        kind: input.kind,
        recipient: input.recipient.into_recipient()?,
        params: input.params,
        link: input.link,
        locale: input.locale,
        dedupe_key: input.dedupe_key,
    };
    Ok(Data { data: service.send_manual(&tenant, &principal, notification).await? })
}

/// One message of the send log (one notification on one channel).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = NotificationMessage)]
pub(crate) struct MessageResponse {
    id: Uuid,
    kind: String,
    category: String,
    channel: String,
    user_id: Option<Uuid>,
    /// Email or phone number; `null` for push and in-app, and after the retention time.
    address: Option<String>,
    locale: String,
    /// `queued`, `deferred` (waiting for the allowed hours), `sent`, `failed`, `skipped` or
    /// `canceled` (consent withdrawn before sending).
    status: String,
    /// Why it was skipped or canceled (see `NotificationSkipReason`); empty otherwise.
    skip_reason: String,
    /// Deferred until then.
    not_before: Option<Timestamp>,
    attempts: i32,
    provider_reference: Option<String>,
    /// Why it failed (error code of the provider, `unsubscribe_url_missing`, `zns_template_missing`…).
    error_code: Option<String>,
    dedupe_key: String,
    created_at: Timestamp,
    sent_at: Option<Timestamp>,
}

impl From<message::Model> for MessageResponse {
    fn from(row: message::Model) -> Self {
        MessageResponse {
            id: row.id,
            kind: row.kind,
            category: row.category,
            channel: row.channel,
            user_id: row.user_id,
            address: row.address,
            locale: row.locale,
            status: row.status,
            skip_reason: row.skip_reason,
            not_before: row.not_before,
            attempts: row.attempts,
            provider_reference: row.provider_reference,
            error_code: row.error_code,
            dedupe_key: row.dedupe_key,
            created_at: row.created_at,
            sent_at: row.sent_at,
        }
    }
}

/// Send log filters.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct MessageQuery {
    /// Only messages of this notification kind, e.g. `orders.order_shipped`.
    kind: Option<String>,
    /// `queued`, `deferred`, `sent`, `failed`, `skipped` or `canceled`.
    status: Option<String>,
    /// e.g. `email`.
    channel: Option<String>,
    /// Only messages to this user.
    user_id: Option<Uuid>,
}

/// List the send log.
///
/// Requires `notifications.messages.read` (it shows addresses).
#[utoipa::path(
    get,
    path = "/messages",
    extensions(("x-permissions" = json!(["notifications.messages.read"]))),
    operation_id = "list_notification_messages",
    security(("bearer" = [])),
    params(MessageQuery, PageQuery),
    responses(
        (status = 200, description = "One page of messages, newest first.", body = Page<MessageResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `notifications.messages.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A filter or paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_messages(
    State(service): State<Arc<NotificationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(filter): Query<MessageQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<MessageResponse>, Error> {
    principal.require(&crate::permissions::MESSAGES_READ)?;
    let filter = MessageFilter {
        kind: filter.kind,
        status: filter.status,
        channel: filter.channel,
        user_id: filter.user_id,
    };
    let page =
        service.list_messages(&tenant, &principal, filter, &PageRequest::try_from(page)?).await?;
    Ok(page.map(MessageResponse::from))
}

/// Get one message of the send log.
///
/// Requires `notifications.messages.read`.
#[utoipa::path(
    get,
    path = "/messages/{id}",
    extensions(("x-permissions" = json!(["notifications.messages.read"]))),
    operation_id = "get_notification_message",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the message.")),
    responses(
        (status = 200, description = "The message.", body = Data<MessageResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `notifications.messages.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such message; code `notifications.message_not_found`.", body = Problem),
    )
)]
async fn get_message(
    State(service): State<Arc<NotificationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<MessageResponse>, Error> {
    Ok(Data { data: service.get_message(&tenant, &principal, id).await?.into() })
}

// ---- Thông báo của nền tảng ----------------------------------------------------------------------

/// A notice from the platform to the admins of every tenant.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = PlatformNoticeRequest)]
pub(crate) struct PlatformNoticeRequest {
    /// `transactional` (admins may turn it off) or `security` (always delivered).
    #[schema(example = "transactional")]
    category: String,
    #[schema(example = "Scheduled maintenance on Sunday")]
    title: String,
    body: String,
    link: Option<String>,
}

/// A queued platform notice.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = PlatformNotice)]
pub(crate) struct PlatformNoticeResponse {
    notice_id: Uuid,
}

/// Send a notice to the admins of every tenant.
///
/// Requires `notifications.platform_notices.send` and an account of the platform tenant. Goes in
/// the background, tenant by tenant, to the inbox and email of every active admin; running again
/// never sends twice to the same person.
#[utoipa::path(
    post,
    path = "/platform-notices",
    extensions(("x-permissions" = json!(["notifications.platform_notices.send"]))),
    operation_id = "send_platform_notice",
    security(("bearer" = [])),
    request_body = PlatformNoticeRequest,
    responses(
        (status = 202, description = "Queued.", body = Data<PlatformNoticeResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `notifications.platform_notices.send` (`permission_denied`), or not an account of the platform tenant (`tenant_platform_only`).", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn send_platform_notice(
    State(service): State<Arc<NotificationsService>>,
    principal: Principal,
    ValidJson(input): ValidJson<PlatformNoticeRequest>,
) -> Result<(StatusCode, Data<PlatformNoticeResponse>), Error> {
    let notice = NewPlatformNotice {
        category: input.category,
        title: input.title,
        body: input.body,
        link: input.link,
    };
    let notice_id = service.queue_platform_notice(&principal, notice).await?;
    Ok((StatusCode::ACCEPTED, Data { data: PlatformNoticeResponse { notice_id } }))
}
