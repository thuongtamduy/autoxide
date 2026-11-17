//! Tool MCP của `notifications`: cùng service, cùng permission với REST.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::mcp::{NoInput, PageInput, Tool};
use autoxide_kernel::pagination::Page;
use autoxide_kernel::tenancy::TenantContext;
use serde::Deserialize;
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::api::Notification;
use crate::custom::Source;
use crate::http_admin::{MessageResponse, RecipientRequest};
use crate::http_me::{DeviceTokenResponse, InboxItem, KindResponse, describe_kind};
use crate::http_templates::{
    DraftRequest, DraftResponse, PreviewResponse, SchemaResponse, describe_schema, parse_source,
};
use crate::messages::MessageFilter;
use crate::permissions::{MESSAGES_READ, SEND, TEMPLATES_MANAGE, TEMPLATES_READ};
use crate::service::NotificationsService;

/// A notification to send (flat fields: give exactly one recipient).
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SendInput {
    /// Key of a declared kind (see `list_kinds`).
    kind: String,
    /// A user of the tenant.
    user_id: Option<Uuid>,
    /// An email address.
    email: Option<String>,
    /// A phone number, e.g. `+84912345678`.
    phone: Option<String>,
    /// `true`: every active admin of the tenant.
    #[serde(default)]
    tenant_admins: bool,
    /// Values for the `{name}` places of the text.
    #[serde(default)]
    #[schema(value_type = Object)]
    params: Value,
    /// Page opened from the notification.
    link: Option<String>,
    /// `en` or `vi`.
    locale: Option<String>,
    /// Required: retrying with the same key sends nothing new.
    dedupe_key: String,
}

/// Which messages.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MessagesInput {
    kind: Option<String>,
    /// `queued`, `deferred`, `sent`, `failed`, `skipped` or `canceled`.
    status: Option<String>,
    /// e.g. `email`.
    channel: Option<String>,
    user_id: Option<Uuid>,
    /// Maximum number of messages, 1 to 200. Defaults to 50.
    limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    cursor: Option<String>,
}

/// Which inbox page.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct InboxInput {
    /// `true`: only unread notifications.
    #[serde(default)]
    unread: bool,
    /// Maximum number of notifications, 1 to 200. Defaults to 50.
    limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    cursor: Option<String>,
}

/// Which notification to mark read.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadInput {
    /// Id of one notification of your inbox; omit to mark all as read.
    id: Option<Uuid>,
}

/// Which template (kind, channel, language).
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SchemaInput {
    /// Key of a declared kind (see `list_kinds`).
    kind: String,
}

/// A template to preview.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreviewInput {
    kind: String,
    /// One channel of the kind, e.g. `email`.
    channel: String,
    /// `en` or `vi`.
    locale: String,
    /// `draft` (default), `published` or `version`.
    source: Option<String>,
    /// With `source = version`.
    version: Option<i32>,
    /// Values to render with (only variables of the kind); omitted → the sample data.
    #[schema(value_type = Option<Object>)]
    data: Option<Value>,
}

/// A template draft to save.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct DraftInput {
    kind: String,
    channel: String,
    /// `en` or `vi`.
    locale: String,
    /// Email subject, push and inbox title.
    #[serde(default)]
    subject: String,
    /// Plain text (email text part, inbox).
    #[serde(default)]
    body_text: String,
    /// HTML of the email body (`email` only).
    body_html: Option<String>,
    /// SMS, ZNS and push text.
    #[serde(default)]
    short: String,
    /// Approved Zalo ZNS template id.
    zns_template_id: Option<String>,
}

/// What was marked read.
#[derive(Debug, serde::Serialize)]
pub(crate) struct Marked {
    updated: u64,
}

pub(crate) fn all(service: &Arc<NotificationsService>) -> Vec<Tool> {
    let (kinds, send, messages, inbox, read) =
        (service.clone(), service.clone(), service.clone(), service.clone(), service.clone());
    let (schema, preview, draft) = (service.clone(), service.clone(), service.clone());
    let devices = service.clone();
    vec![
        Tool::new(
            "list_my_devices",
            "List your mobile devices registered for push notifications (platform, app, last use); \
             tokens are never shown.",
            move |tenant: TenantContext, principal: Principal, _: NoInput| {
                let service = devices.clone();
                async move {
                    let rows = service.my_device_tokens(&tenant, &principal).await?;
                    Ok::<Vec<DeviceTokenResponse>, Error>(rows.into_iter().map(DeviceTokenResponse::from).collect())
                }
            },
        )
        .for_any_signed_in()
        .read_only(),
        Tool::new(
            "get_template_schema",
            "Show the variables a notification template of a kind may use (`{{ path }}`), which are \
             required, and sample data.",
            move |_tenant: TenantContext, _principal: Principal, input: SchemaInput| {
                let service = schema.clone();
                async move { describe_schema(&service, &input.kind) as Result<SchemaResponse, Error> }
            },
        )
        .for_any_signed_in()
        .read_only(),
        Tool::new(
            "preview_template",
            "Render a notification template (the draft by default, or the published or an older \
             version) with the sample data of the kind or your `data`. Nothing is sent.",
            move |tenant: TenantContext, principal: Principal, input: PreviewInput| {
                let service = preview.clone();
                async move {
                    crate::custom::require_read(&principal)?;
                    let source: Source = parse_source(input.source.as_deref(), input.version)?;
                    let target = (input.kind.as_str(), input.channel.as_str(), input.locale.as_str());
                    let rendered =
                        service.preview_template(&tenant, &principal, target, source, input.data).await?;
                    Ok::<PreviewResponse, Error>(rendered.into())
                }
            },
        )
        .requires(TEMPLATES_READ)
        .or_requires(TEMPLATES_MANAGE)
        .read_only(),
        Tool::new(
            "update_template_draft",
            "Save the draft of a notification template (template engine syntax: `{{ name }}`, \
             `{% if %}`, `{% for %}`). Checked against the variables of the kind; every problem is \
             reported at once. A draft is not used until a person publishes it over REST.",
            move |tenant: TenantContext, principal: Principal, input: DraftInput| {
                let service = draft.clone();
                async move {
                    let target = (input.kind.as_str(), input.channel.as_str(), input.locale.as_str());
                    let row = service.put_draft(&tenant, &principal, target, DraftRequest {
                        subject: input.subject,
                        body_text: input.body_text,
                        body_html: input.body_html,
                        short: input.short,
                        zns_template_id: input.zns_template_id,
                    }.into()).await?;
                    Ok::<DraftResponse, Error>(row.into())
                }
            },
        )
        .requires(TEMPLATES_MANAGE),
        Tool::new(
            "list_kinds",
            "List the notification kinds declared by modules: key, category (`security`, \
             `transactional`, `marketing`) and channels.",
            move |_tenant: TenantContext, _principal: Principal, _: NoInput| {
                let service = kinds.clone();
                async move {
                    Ok::<Vec<KindResponse>, Error>(service.kinds.all().iter().map(describe_kind).collect())
                }
            },
        )
        .for_any_signed_in()
        .read_only(),
        Tool::new(
            "send_notification",
            "Send a notification of a declared kind to one recipient (user, email, phone or every \
             admin). Advertising needs `notifications.send_marketing` and is only sent to people who \
             consented. Always pass a `dedupe_key`: retrying with it sends nothing new.",
            move |tenant: TenantContext, principal: Principal, input: SendInput| {
                let service = send.clone();
                async move {
                    principal.require(&SEND)?;
                    let recipient = RecipientRequest {
                        user_id: input.user_id,
                        email: input.email,
                        phone: input.phone,
                        tenant_admins: input.tenant_admins,
                    }
                    .into_recipient()?;
                    let notification = Notification {
                        kind: input.kind,
                        recipient,
                        params: input.params,
                        link: input.link,
                        locale: input.locale,
                        dedupe_key: input.dedupe_key,
                    };
                    service.send_manual(&tenant, &principal, notification).await
                }
            },
        )
        .requires(SEND),
        Tool::new(
            "list_messages",
            "List the send log of notifications (one row per channel), newest first, with status and \
             skip reason. Shows addresses of recipients.",
            move |tenant: TenantContext, principal: Principal, input: MessagesInput| {
                let service = messages.clone();
                async move {
                    let page = PageInput::new(input.limit, input.cursor, None).page()?;
                    let filter = MessageFilter {
                        kind: input.kind,
                        status: input.status,
                        channel: input.channel,
                        user_id: input.user_id,
                    };
                    let found = service.list_messages(&tenant, &principal, filter, &page).await?;
                    Ok::<Page<MessageResponse>, Error>(found.map(MessageResponse::from))
                }
            },
        )
        .requires(MESSAGES_READ)
        .read_only(),
        Tool::new(
            "list_my_inbox",
            "List your in-app notifications, newest first.",
            move |tenant: TenantContext, principal: Principal, input: InboxInput| {
                let service = inbox.clone();
                async move {
                    let page = PageInput::new(input.limit, input.cursor, None).page()?;
                    let found = service.inbox(&tenant, &principal, input.unread, &page).await?;
                    Ok::<Page<InboxItem>, Error>(found.map(InboxItem::from))
                }
            },
        )
        .for_any_signed_in()
        .read_only(),
        Tool::new(
            "mark_inbox_read",
            "Mark one of your in-app notifications as read, or all of them when `id` is omitted.",
            move |tenant: TenantContext, principal: Principal, input: ReadInput| {
                let service = read.clone();
                async move {
                    let updated = match input.id {
                        Some(id) => {
                            service.mark_read(&tenant, &principal, id).await?;
                            1
                        }
                        None => service.mark_all_read(&tenant, &principal).await?,
                    };
                    Ok::<Marked, Error>(Marked { updated })
                }
            },
        )
        .for_any_signed_in(),
    ]
}
