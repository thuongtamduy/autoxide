//! Route REST của chính người dùng (dưới `/api/v1/notifications`): loại thông báo, hộp thư, tuỳ chọn
//! nhận, đăng ký đẩy; và hai route công khai (khoá web push, rút đồng ý bằng link).

use autoxide_kernel::time::Timestamp;
use std::collections::HashMap;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{NaturalOrderPageQuery, Page, PageQuery, PageRequest, page_of};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::{OptionalTenant, TenantContext};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::api::{Category, Channel, NotificationKind};
use crate::entity::{device_token, inbox, preference, push_subscription};
use crate::me::{
    NewDeviceToken, NewPushSubscription, PreferenceChange, device_token_not_found,
    push_subscription_not_found,
};
use crate::service::NotificationsService;

pub(crate) fn routes(service: Arc<NotificationsService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_kinds))
        .routes(routes!(my_inbox))
        .routes(routes!(unread_count))
        .routes(routes!(mark_read))
        .routes(routes!(mark_all_read))
        .routes(routes!(my_preferences, set_my_preferences))
        .routes(routes!(add_push_subscription))
        .routes(routes!(remove_push_subscription))
        .routes(routes!(list_device_tokens, add_device_token))
        .routes(routes!(remove_device_token))
        .routes(routes!(web_push_key))
        .routes(routes!(unsubscribe))
        .with_state(service)
}

// ---- Loại thông báo ------------------------------------------------------------------------------

/// A notification kind declared by a module.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = NotificationKindInfo)]
pub(crate) struct KindResponse {
    /// `<module>.<name>`, e.g. `orders.order_shipped`.
    key: String,
    category: Category,
    /// Channels this kind may go to.
    channels: Vec<Channel>,
    /// `true` for security notifications: they cannot be turned off.
    locked: bool,
    /// Advertising: the consent purpose it needs (in `compliance`).
    purpose: Option<String>,
    /// Advertising invitation: sent at most once per address and channel.
    invitation: bool,
}

pub(crate) fn describe_kind(kind: &NotificationKind) -> KindResponse {
    KindResponse {
        key: kind.key().to_owned(),
        category: kind.category,
        channels: kind.channels.to_vec(),
        locked: kind.category == Category::Security,
        purpose: (kind.category == Category::Marketing).then(|| kind.consent_purpose().to_owned()),
        invitation: kind.invitation,
    }
}

/// List the notification kinds.
///
/// Any signed-in user. For a preferences page: every kind, its category and channels. By key
/// unless `order=desc`; paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/kinds",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_notification_kinds",
    security(("bearer" = [])),
    params(NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the declared kinds, by key.", body = Page<KindResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_kinds(
    State(service): State<Arc<NotificationsService>>,
    _principal: Principal,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<KindResponse>, Error> {
    kind_page(&service, &PageRequest::try_from(page)?)
}

/// Một trang loại thông báo theo khoá (REST `GET /kinds` và tool `notifications.list_kinds` cùng gọi). Loại do
/// module khai trong code (sổ loại), không nằm trong database: `page_of` theo khoá (duy nhất).
pub(crate) fn kind_page(
    service: &NotificationsService,
    page: &PageRequest,
) -> Result<Page<KindResponse>, Error> {
    let kinds = service.kinds.all().iter().map(describe_kind).collect();
    page_of(kinds, |kind: &KindResponse| kind.key.clone(), page)
}

// ---- Hộp thư -------------------------------------------------------------------------------------

/// A notification in your in-app inbox.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = NotificationInboxItem)]
pub(crate) struct InboxItem {
    id: Uuid,
    kind: String,
    category: String,
    title: String,
    body: String,
    link: Option<String>,
    /// When you read it; `null`: unread.
    read_at: Option<Timestamp>,
    created_at: Timestamp,
}

impl From<inbox::Model> for InboxItem {
    fn from(row: inbox::Model) -> Self {
        InboxItem {
            id: row.id,
            kind: row.kind,
            category: row.category,
            title: row.title,
            body: row.body,
            link: row.link,
            read_at: row.read_at,
            created_at: row.created_at,
        }
    }
}

/// Inbox filter.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct InboxQuery {
    /// `true`: only unread notifications.
    #[serde(default)]
    unread: bool,
}

/// List your in-app notifications.
///
/// Any signed-in user; newest first.
#[utoipa::path(
    get,
    path = "/me/inbox",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_my_notifications",
    security(("bearer" = [])),
    params(InboxQuery, PageQuery),
    responses(
        (status = 200, description = "One page of your notifications.", body = Page<InboxItem>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn my_inbox(
    State(service): State<Arc<NotificationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(filter): Query<InboxQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<InboxItem>, Error> {
    let page =
        service.inbox(&tenant, &principal, filter.unread, &PageRequest::try_from(page)?).await?;
    Ok(page.map(InboxItem::from))
}

/// How many notifications you have not read.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = NotificationUnreadCount)]
pub(crate) struct UnreadCount {
    unread: u64,
}

/// Count your unread notifications.
///
/// Any signed-in user. Cheap enough to poll for a badge.
#[utoipa::path(
    get,
    path = "/me/inbox/unread-count",
    extensions(("x-permissions" = json!([]))),
    operation_id = "count_my_unread_notifications",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The count.", body = Data<UnreadCount>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
    )
)]
async fn unread_count(
    State(service): State<Arc<NotificationsService>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<UnreadCount>, Error> {
    Ok(Data { data: UnreadCount { unread: service.unread_count(&tenant, &principal).await? } })
}

/// Mark one of your notifications as read.
///
/// Any signed-in user. Calling again changes nothing.
#[utoipa::path(
    post,
    path = "/me/inbox/{id}/read",
    extensions(("x-permissions" = json!([]))),
    operation_id = "mark_my_notification_read",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the notification.")),
    responses(
        (status = 200, description = "The notification, read.", body = Data<InboxItem>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "Not in your inbox; code `notifications.inbox_not_found`.", body = Problem),
    )
)]
async fn mark_read(
    State(service): State<Arc<NotificationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<InboxItem>, Error> {
    Ok(Data { data: service.mark_read(&tenant, &principal, id).await?.into() })
}

/// How many notifications were marked read.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = NotificationReadAll)]
pub(crate) struct ReadAll {
    updated: u64,
}

/// Mark all your notifications as read.
///
/// Any signed-in user.
#[utoipa::path(
    post,
    path = "/me/inbox/read-all",
    extensions(("x-permissions" = json!([]))),
    operation_id = "mark_all_my_notifications_read",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "How many were unread.", body = Data<ReadAll>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
    )
)]
async fn mark_all_read(
    State(service): State<Arc<NotificationsService>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<ReadAll>, Error> {
    Ok(Data { data: ReadAll { updated: service.mark_all_read(&tenant, &principal).await? } })
}

// ---- Tuỳ chọn nhận -------------------------------------------------------------------------------

/// Whether you get a kind on one channel.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = NotificationChannelPreference)]
pub(crate) struct ChannelPreference {
    channel: Channel,
    enabled: bool,
    /// Security notifications cannot be turned off.
    locked: bool,
}

/// Your choices for one kind (a kind setting wins over the category setting; no setting: on).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = NotificationKindPreference)]
pub(crate) struct KindPreference {
    kind: String,
    category: Category,
    channels: Vec<ChannelPreference>,
}

fn effective(kinds: &[NotificationKind], rows: &[preference::Model]) -> Vec<KindPreference> {
    let stored: HashMap<(&str, &str), bool> =
        rows.iter().map(|row| ((row.target.as_str(), row.channel.as_str()), row.enabled)).collect();
    kinds
        .iter()
        .map(|kind| {
            let kind_target = format!("kind:{}", kind.key());
            let category_target = format!("category:{}", kind.category.as_str());
            let locked = kind.category == Category::Security;
            let channels = kind
                .channels
                .iter()
                .map(|channel| {
                    let name = channel.as_str();
                    let enabled = locked
                        || stored
                            .get(&(kind_target.as_str(), name))
                            .or_else(|| stored.get(&(category_target.as_str(), name)))
                            .copied()
                            .unwrap_or(true);
                    ChannelPreference { channel: *channel, enabled, locked }
                })
                .collect();
            KindPreference { kind: kind.key().to_owned(), category: kind.category, channels }
        })
        .collect()
}

/// One change of your choices.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = NotificationPreferenceChange)]
pub(crate) struct PreferenceItem {
    /// `kind:<key>` (one kind) or `category:<security|transactional|marketing>` (every kind of it).
    #[schema(example = "category:marketing")]
    target: String,
    #[schema(example = "email")]
    channel: String,
    enabled: bool,
}

/// Changes of your choices.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = NotificationPreferencesRequest)]
pub(crate) struct PreferencesRequest {
    items: Vec<PreferenceItem>,
}

/// List which notifications you get.
///
/// Any signed-in user. Every kind with each of its channels, on or off. By kind unless
/// `order=desc`; paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/me/preferences",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_my_notification_preferences",
    security(("bearer" = [])),
    params(NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of your choices per kind and channel, by kind.", body = Page<KindPreference>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn my_preferences(
    State(service): State<Arc<NotificationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<KindPreference>, Error> {
    let page = PageRequest::try_from(page)?;
    let rows = service.my_preferences(&tenant, &principal).await?;
    // Một dòng mỗi loại thông báo (sổ loại trong code), ghép với lựa chọn đã lưu của người gọi (vài dòng):
    // danh sách dựng từ code nên `page_of` theo khoá loại.
    page_of(effective(&service.kinds.all(), &rows), |item| item.kind.clone(), &page)
}

/// Choose which notifications you get.
///
/// Any signed-in user. Advertising also needs your consent (`PUT /api/v1/compliance/me/consents`);
/// turning it off here stops it even with a consent.
#[utoipa::path(
    put,
    path = "/me/preferences",
    extensions(("x-permissions" = json!([]))),
    operation_id = "set_my_notification_preferences",
    security(("bearer" = [])),
    request_body = PreferencesRequest,
    responses(
        (status = 200, description = "Your choices after the change.", body = Data<Vec<KindPreference>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "A value is invalid (every problem in `errors`), or a security notification would be turned off (`notifications.preference_locked`).", body = Problem),
    )
)]
async fn set_my_preferences(
    State(service): State<Arc<NotificationsService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<PreferencesRequest>,
) -> Result<Data<Vec<KindPreference>>, Error> {
    let changes = input
        .items
        .into_iter()
        .map(|item| PreferenceChange {
            target: item.target,
            channel: item.channel,
            enabled: item.enabled,
        })
        .collect();
    let rows = service.set_preferences(&tenant, &principal, changes).await?;
    Ok(Data { data: effective(&service.kinds.all(), &rows) })
}

// ---- Đẩy -----------------------------------------------------------------------------------------

/// Keys of a browser push subscription.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = NotificationPushKeys)]
pub(crate) struct PushKeys {
    /// P-256 public key of the browser (base64url).
    p256dh: String,
    /// Authentication secret (base64url).
    auth: String,
}

/// A browser push subscription (`PushSubscription.toJSON()` of the browser).
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = NotificationPushSubscriptionRequest)]
pub(crate) struct PushSubscriptionRequest {
    #[schema(example = "https://fcm.googleapis.com/fcm/send/abc")]
    endpoint: String,
    keys: PushKeys,
    /// Ignored when sent (some browsers add it).
    #[serde(rename = "expirationTime", default)]
    #[schema(value_type = Option<i64>)]
    expiration_time: Option<serde_json::Value>,
    /// Shown in your device list.
    user_agent: Option<String>,
}

/// A registered browser.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = NotificationPushSubscription)]
pub(crate) struct PushSubscriptionResponse {
    id: Uuid,
    endpoint: String,
    user_agent: Option<String>,
    created_at: Timestamp,
    last_used_at: Option<Timestamp>,
}

impl From<push_subscription::Model> for PushSubscriptionResponse {
    fn from(row: push_subscription::Model) -> Self {
        PushSubscriptionResponse {
            id: row.id,
            endpoint: row.endpoint,
            user_agent: row.user_agent,
            created_at: row.created_at,
            last_used_at: row.last_used_at,
        }
    }
}

/// Register this browser for push notifications.
///
/// Any signed-in user. Registering the same browser again updates it.
#[utoipa::path(
    post,
    path = "/me/push-subscriptions",
    extensions(("x-permissions" = json!([]))),
    operation_id = "add_my_push_subscription",
    security(("bearer" = [])),
    request_body = PushSubscriptionRequest,
    responses(
        (status = 201, description = "The subscription.", body = Data<PushSubscriptionResponse>, headers(("Location" = String, description = "Path of the subscription."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "Not a valid subscription; code `notifications.push_subscription_invalid`.", body = Problem),
    )
)]
async fn add_push_subscription(
    State(service): State<Arc<NotificationsService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<PushSubscriptionRequest>,
) -> Result<Created<PushSubscriptionResponse>, Error> {
    let _ = input.expiration_time;
    let row = service
        .add_push_subscription(
            &tenant,
            &principal,
            NewPushSubscription {
                endpoint: input.endpoint,
                p256dh: input.keys.p256dh,
                auth: input.keys.auth,
                user_agent: input.user_agent,
            },
        )
        .await?;
    Ok(Created::new(format!("/notifications/me/push-subscriptions/{}", row.id), row.into()))
}

/// Stop push notifications to a browser.
///
/// Any signed-in user, for their own subscriptions.
#[utoipa::path(
    delete,
    path = "/me/push-subscriptions/{id}",
    extensions(("x-permissions" = json!([]))),
    operation_id = "remove_my_push_subscription",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the subscription.")),
    responses(
        (status = 204, description = "Removed."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "Not one of your subscriptions; code `notifications.push_subscription_not_found`.", body = Problem),
    )
)]
async fn remove_push_subscription(
    State(service): State<Arc<NotificationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    let removed = service
        .remove_push_subscription(
            &tenant,
            Actor::from(&principal),
            id,
            principal.user_id(),
            "removed",
        )
        .await?;
    if removed { Ok(StatusCode::NO_CONTENT) } else { Err(push_subscription_not_found()) }
}

/// A mobile device to register.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = NotificationDeviceTokenRequest)]
pub(crate) struct DeviceTokenRequest {
    /// `ios` or `android`.
    platform: String,
    /// The push token from FCM or APNs.
    token: String,
    /// Bundle id / package name of the app.
    app_id: String,
    /// `en` or `vi`.
    locale: Option<String>,
}

/// A registered mobile device (the token is never shown again).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = NotificationDeviceToken)]
pub(crate) struct DeviceTokenResponse {
    id: Uuid,
    platform: String,
    app_id: String,
    locale: Option<String>,
    created_at: Timestamp,
    last_used_at: Option<Timestamp>,
}

impl From<device_token::Model> for DeviceTokenResponse {
    fn from(row: device_token::Model) -> Self {
        DeviceTokenResponse {
            id: row.id,
            platform: row.platform,
            app_id: row.app_id,
            locale: row.locale,
            created_at: row.created_at,
            last_used_at: row.last_used_at,
        }
    }
}

/// List your registered mobile devices.
///
/// Any signed-in user, for their own devices. Tokens are stored encrypted and never returned.
/// Newest first unless `order=asc`; paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/me/device-tokens",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_my_device_tokens",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of your devices, newest first.", body = Page<DeviceTokenResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_device_tokens(
    State(service): State<Arc<NotificationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<DeviceTokenResponse>, Error> {
    let page = PageRequest::try_from(page)?;
    let rows = service.my_device_tokens(&tenant, &principal, &page).await?;
    Ok(rows.map(DeviceTokenResponse::from))
}

/// Register this mobile device for push notifications.
///
/// Any signed-in user. Registering the same token again updates it.
#[utoipa::path(
    post,
    path = "/me/device-tokens",
    extensions(("x-permissions" = json!([]))),
    operation_id = "add_my_device_token",
    security(("bearer" = [])),
    request_body = DeviceTokenRequest,
    responses(
        (status = 201, description = "The device.", body = Data<DeviceTokenResponse>, headers(("Location" = String, description = "Path of the device."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn add_device_token(
    State(service): State<Arc<NotificationsService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<DeviceTokenRequest>,
) -> Result<Created<DeviceTokenResponse>, Error> {
    let row = service
        .add_device_token(
            &tenant,
            &principal,
            NewDeviceToken {
                platform: input.platform,
                token: input.token,
                app_id: input.app_id,
                locale: input.locale,
            },
        )
        .await?;
    Ok(Created::new(format!("/notifications/me/device-tokens/{}", row.id), row.into()))
}

/// Stop push notifications to a mobile device.
///
/// Any signed-in user, for their own devices.
#[utoipa::path(
    delete,
    path = "/me/device-tokens/{id}",
    extensions(("x-permissions" = json!([]))),
    operation_id = "remove_my_device_token",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the device.")),
    responses(
        (status = 204, description = "Removed."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "Not one of your devices; code `notifications.device_token_not_found`.", body = Problem),
    )
)]
async fn remove_device_token(
    State(service): State<Arc<NotificationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    let removed = service
        .remove_device_token(&tenant, Actor::from(&principal), id, principal.user_id(), "removed")
        .await?;
    if removed { Ok(StatusCode::NO_CONTENT) } else { Err(device_token_not_found()) }
}

/// The public key browsers subscribe with.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = NotificationWebPushKey)]
pub(crate) struct WebPushKey {
    /// VAPID public key (base64url) for `PushManager.subscribe({applicationServerKey})`.
    public_key: String,
}

/// Get the web push public key.
///
/// No sign-in.
#[utoipa::path(
    get,
    path = "/public/web-push-key",
    extensions(("x-public" = json!(true))),
    operation_id = "get_web_push_key",
    responses(
        (status = 200, description = "The key.", body = Data<WebPushKey>),
        (status = 503, description = "Web push is not set up on this server; code `web_push_disabled`.", body = Problem),
    )
)]
async fn web_push_key(
    State(service): State<Arc<NotificationsService>>,
) -> Result<Data<WebPushKey>, Error> {
    let sender = service.web_push.as_ref().ok_or_else(autoxide_kernel::push::web_push_disabled)?;
    Ok(Data { data: WebPushKey { public_key: sender.public_key() } })
}

/// The token from an advertising email.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = NotificationUnsubscribeRequest)]
pub(crate) struct UnsubscribeRequest {
    token: String,
}

/// Stop advertising with the link of an email.
///
/// No sign-in: the token of the link is the proof. Withdraws the consent the email was sent with
/// (same purpose and channel) and cancels advertising still waiting. Calling again changes nothing.
#[utoipa::path(
    post,
    path = "/public/unsubscribe",
    extensions(("x-public" = json!(true))),
    operation_id = "unsubscribe_from_notifications",
    request_body = UnsubscribeRequest,
    responses(
        (status = 204, description = "Consent withdrawn."),
        (status = 400, description = "Unknown or expired link; code `notifications.link_invalid`.", body = Problem),
    )
)]
async fn unsubscribe(
    State(service): State<Arc<NotificationsService>>,
    tenant: OptionalTenant,
    ValidJson(input): ValidJson<UnsubscribeRequest>,
) -> Result<StatusCode, Error> {
    service.unsubscribe(tenant.get(), &input.token).await?;
    Ok(StatusCode::NO_CONTENT)
}
