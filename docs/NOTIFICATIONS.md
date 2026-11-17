# Notifications

[Tiếng Việt](NOTIFICATIONS.vi.md)

The `notifications` module sends notifications to people over **email, SMS, Zalo ZNS, web push,
mobile push and an in-app inbox**. Modules declare the kinds of notification they send; people choose
which ones they get; advertising follows the consent ledger and the legal rules of the `compliance`
module. Platform staff can send a notice to the admins of every tenant.

> The legal notes below (consent, allowed hours, advertising labels) come from Decree 91/2020/ND-CP
> and the Law on Personal Data Protection 91/2025/QH15 and need checking against the original
> documents by someone qualified before use in production.

## Kinds and categories

Every notification has a **kind** declared by a module, e.g. `orders.order_shipped`. A kind has:

| Field | Meaning |
|---|---|
| category | `security` (sign-in, password: always sent, cannot be turned off), `transactional` (caused by something the person did: on unless they turn it off), `marketing` (advertising: needs a consent) |
| channels | where it may go: `email`, `sms`, `zns`, `web_push`, `in_app`, `mobile_push` |
| default texts | title, body and a short text (SMS, ZNS, push) in English, with `{name}` places; the Vietnamese translation lives in the declaring module's `locales/vi.json` |
| purpose | advertising only: the consent purpose to check (default `marketing`) |
| invitation | advertising only: an invitation to sign up, sent at most once per address and channel |

`GET /api/v1/notifications/kinds` lists every kind (any signed-in user), so an interface can build a
preferences page.

## How a notification is sent

1. A module calls `Notifier::notify` (from an event handler or a job, never inside its own write
   transaction) with the kind, the recipient (a user, an email, a phone number, or every admin of the
   tenant), the values for the text, an optional link and a **required** `dedupe_key`.
2. The module picks the channels: the kind's channels, minus those the person turned off (security
   ignores preferences), minus those without an address (email of the account, phone number, push
   subscriptions), minus those without a provider on this server. Advertising also needs a consent
   in the ledger of `compliance`. Each recipient gets at most `AUTOXIDE_NOTIFICATIONS_PER_RECIPIENT_PER_HOUR`
   (20) transactional and advertising notifications per hour.
3. One message per channel is written to the send log with a delivery job, in one transaction. The
   same `dedupe_key` again returns the same messages and sends nothing new — retry safely.
4. The delivery job reads the message again. For advertising it **checks the consent again** right
   before sending (withdrawn → `skipped` / `consent_withdrawn`) and the allowed hours: outside them the
   message is `deferred` to the next opening, and dropped (`quiet_hours_expired`) if it could not go out
   within 24 hours. It then renders the text (tenant text, else translation, else English) and hands it
   to the provider. Temporary errors are retried; permanent ones mark the message `failed` with an
   `error_code`.

Skipped channels are listed in the result (`skipped`) and in the send log with a reason:
`no_consent`, `consent_withdrawn`, `preference_off`, `no_address`, `channel_disabled`,
`quiet_hours_expired`, `invitation_used`, `throttled`.

## Advertising

- Only to people with a consent for the purpose and channel (`PUT /api/v1/compliance/me/consents`, or a
  module recording one through `ConsentLedger`).
- Allowed hours and labels are **legal rules with effective dates** in `compliance`, not code:
  `notifications.send_windows` (SMS and ZNS 07:00–22:00 Vietnam time, `utc_offset_minutes: 420`) and
  `notifications.marketing_labels` (`QC ` before SMS, `[QC] ` before email subjects and push titles).
  Platform staff import new versions with `POST /api/v1/compliance/rules`.
- Every advertising email carries a `List-Unsubscribe` header and a footer with a link to your
  website's page `AUTOXIDE_NOTIFICATIONS_UNSUBSCRIBE_URL` (with `{token}`). That page sends the token to
  `POST /api/v1/notifications/public/unsubscribe` (no sign-in): the consent is withdrawn and waiting
  advertising is canceled. Without the setting, advertising emails are not sent (`failed` /
  `unsubscribe_url_missing`). The link works while the send log keeps the address
  (`AUTOXIDE_NOTIFICATIONS_RETENTION_DAYS`, 90 days).
- When a consent is withdrawn anywhere (event `compliance.consent_withdrawn`), queued and deferred
  advertising of that person is canceled.

## Channels and providers

| Channel | Provider (kernel trait) | Address |
|---|---|---|
| `email` | `Mailer` (module `mail`) | the account's email, or the email given |
| `sms` | `SmsSender` | the phone number given |
| `zns` | `ZnsSender` | the phone number given; the approved Zalo template id is set per tenant with `PUT /templates/{kind}/zns/{locale}` (`zns_template_id`) |
| `web_push` | `WebPushSender` | the browsers the user registered (`POST /me/push-subscriptions`) |
| `mobile_push` | `MobilePushSender` | the devices the user registered (`POST /me/device-tokens`) |
| `in_app` | built in | the user's inbox (`GET /me/inbox`) |

SMS, ZNS, web push and mobile push providers are separate modules that need the provider's keys; this
release ships none (tests use fakes). Without a provider the channel is skipped (`channel_disabled`).
Push secrets (the browser's `auth` key, device tokens) are stored encrypted. When a push service says a
subscription or token is gone, it is removed.

## For people: inbox and preferences

- `GET /me/inbox` (page, `unread=true` for unread only), `GET /me/inbox/unread-count`,
  `POST /me/inbox/{id}/read`, `POST /me/inbox/read-all`.
- `GET /me/preferences` lists every kind with each channel on or off; `PUT /me/preferences` takes
  `{"items": [{"target": "category:marketing", "channel": "email", "enabled": false}]}`. A target is
  `kind:<key>` or `category:<name>`; a kind setting wins over its category. Turning a security
  notification off answers `notifications.preference_locked`.
- `GET /public/web-push-key` gives the key for `PushManager.subscribe`.

## For admins

| Endpoint | Permission |
|---|---|
| `GET /templates`, `PUT` / `DELETE /templates/{kind}/{channel}/{locale}` — replace a default text (only the `{name}` places of the default text and `{link}`) | `notifications.templates.manage` |
| `POST /messages` — send a notification | `notifications.send` (advertising also `notifications.send_marketing`) |
| `GET /messages`, `GET /messages/{id}` — the send log | `notifications.messages.read` |
| `POST /platform-notices` — a notice to the admins of every tenant (platform staff) | `notifications.platform_notices.send` |

A platform notice goes in the background, 50 tenants at a time, to the inbox and email of every active
admin; running it again never sends twice. `category` is `transactional` (admins may turn it off) or
`security`. The `plans` module uses the same path to tell tenant admins when their subscription is
suspended or past due, and once a month when they use 80% of a limit.

MCP tools: `notifications.list_kinds`, `notifications.send_notification`, `notifications.list_messages`,
`notifications.list_my_inbox`, `notifications.mark_inbox_read`.

## Declaring a kind in a module

Register the module **after** `notifications` in `crates/app/src/modules.rs`, then:

```rust
use autoxide_notifications::api::{Category, Channel, Notification, NotificationKind, NotificationTemplate, Notifier, Recipient};

const SHIPPED: NotificationKind = NotificationKind::new(
    Category::Transactional,
    &[Channel::Email, Channel::InApp],
    NotificationTemplate::new(
        "orders.order_shipped",
        "Order {code} is on its way",
        "Your order {code} was handed to the carrier.",
        "Order {code} shipped",
    ),
);

// In Module::start:
let notifier = context.require::<dyn Notifier>().ok();
if let Some(notifier) = &notifier {
    notifier.declare(SHIPPED)?;
}

// In an event handler or a job:
let notification = Notification::new(SHIPPED.key(), Recipient::User(user_id), json!({"code": code}), event.id.to_string())
    .link(format!("/orders/{id}"));
notifier.notify(tenant_id, notification).await?;
```

Add `orders.order_shipped.title`, `.body` and `.short` to the module's `locales/vi.json`
(`cargo xtask check` asks for them). Advertising: `Category::Marketing`, optionally `.purpose("…")`;
an invitation to sign up: `.invitation()`.

## Settings

| Variable | Default | Meaning |
|---|---|---|
| `AUTOXIDE_NOTIFICATIONS_RETENTION_DAYS` | 90 | days the address and values of a message are kept in the send log |
| `AUTOXIDE_NOTIFICATIONS_INBOX_RETENTION_DAYS` | 180 | days a notification stays in the inbox |
| `AUTOXIDE_NOTIFICATIONS_UNSUBSCRIBE_URL` | — | your page that withdraws a consent, with `{token}` |
| `AUTOXIDE_NOTIFICATIONS_PER_RECIPIENT_PER_HOUR` | 20 | transactional and advertising notifications per recipient per hour |

A daily job (`notifications.purge`) applies the retention times. A person's inbox, preferences, devices and
send log are part of their personal data export; when their account is deleted they are removed (the send log
keeps the rows without address or values).
