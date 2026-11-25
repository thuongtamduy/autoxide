# Sending SMS through eSMS

The `esms` module sends one-time codes and customer-care SMS through [eSMS](https://esms.vn), a
Vietnamese SMS brandname provider. Each tenant uses **its own** eSMS account and brandname.

> **Experimental.** This adapter was written from the provider's public documentation
> (<https://developers.esms.vn>, read on 2026-11-21) and is not yet verified against the live service.
> Test it with the eSMS test environment (below) before relying on it.

Vietnamese: [`esms.vi.md`](esms.vi.md).

## What it does, and what it does not

| | |
| --- | --- |
| One-time codes (phone verification, sign-in, order and booking codes) | sent as eSMS customer-care messages (`SmsType` 2) |
| Transactional notices of the `notifications` module | sent the same way |
| Advertising messages | **refused** with `esms.marketing_unsupported` |
| Numbers outside Vietnam (not `+84`) | refused with `esms.phone_unsupported` |
| Delivery reports (did the message reach the phone) | not read; an accepted message means eSMS took it |

Advertising SMS in Vietnam needs prior consent, the do-not-call list, permitted hours and an
advertising brandname (Decree 91/2020/ND-CP; check the original text or ask a legal professional).
eSMS has a separate bulk function for it that takes at least 30 numbers per request. The adapter does
not send advertising through the customer-care route.

## 1. For the operator: turn the module on

The module is experimental, so it is off until named:

```
AUTOXIDE_MODULES=official,esms
```

`AUTOXIDE_ESMS_API_URL` (default `https://rest.esms.vn`) only needs changing to point at a test
server. There is no server-wide eSMS account.

When the module is on, every tenant sends SMS through its own eSMS account; a tenant that has not
set one gets the error `provider.not_configured` where an SMS would be sent.

## 2. For the tenant: what to get from eSMS

1. Register at [esms.vn](https://esms.vn) and sign in.
2. Open **API management** ("Quản lý API") and copy the **ApiKey** and the **SecretKey**.
3. Ask eSMS to register your **brandname** for customer-care messages, and the **message
   templates** you will send. The mobile networks only deliver customer-care messages that match a
   registered template; anything else is refused (`esms.template_rejected`).
4. If eSMS restricts your account by IP address, give them the address of your Autoxide server
   (otherwise: `esms.ip_not_allowed`).

Every SMS that carries a one-time code has a default text, and a tenant setting that replaces it so it
matches the template you registered (group `sms_template`, list them with
`GET /api/v1/kernel/settings?group=sms_template`). Register either the default text (replace the parts in
braces by the variable syntax eSMS asks for) or your own text, then set your text in the setting:

| Used for | Setting | Default text | Placeholders |
| --- | --- | --- | --- |
| Phone verification | `compliance.sms_template.phone_verification` | `Ma xac thuc cua ban: {code}. Het han sau {minutes} phut. Khong chia se ma nay.` | `{code}`, `{minutes}` |
| Sign-in by phone | `compliance.sms_template.sign_in` | `Ma dang nhap cua ban: {code}. Het han sau {minutes} phut. Khong chia se ma nay.` | `{code}`, `{minutes}` |
| Booking code (`bookings`) | `bookings.sms_template.guest_code` | `Your booking code is {code}. It expires in {minutes} minutes.` | `{code}`, `{minutes}` |
| Guest order lookup (`orders`) | `orders.sms_template.order_lookup` | `Your code to view order {number} is {code}. It expires in {minutes} minutes.` (translated to the order's language) | `{code}`, `{number}`, `{minutes}` |
| Guest address change (`orders`) | `orders.sms_template.address_change` | `Your code to change the delivery address of order {number} is {code}. It expires in {minutes} minutes.` (translated) | `{code}`, `{number}`, `{minutes}` |
| Notices (`notifications`) | — | the texts of your own templates in that module | — |

```bash
curl -X PUT https://your-server/api/v1/kernel/settings/compliance.sms_template.sign_in \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"value": "[MYSHOP] {code} la ma dang nhap cua ban, hieu luc {minutes} phut"}'
```

A template must contain `{code}` exactly once and only the placeholders of its message (otherwise 422,
error on `value`). Your text is sent as is, whatever the buyer's language. `DELETE` the setting to go
back to the default text.

## 3. For the tenant: enter the account

All calls need the permission `kernel.settings.manage`. Secrets are set over REST only; they are
stored encrypted and never returned.

```bash
curl -X PUT https://your-server/api/v1/kernel/settings/esms.connection.sms.api_key \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"value": "YOUR_API_KEY"}'
```

| Setting | Secret | Value |
| --- | --- | --- |
| `esms.connection.sms.api_key` | yes | ApiKey |
| `esms.connection.sms.secret_key` | yes | SecretKey |
| `esms.connection.sms.brandname` | no | your registered brandname |
| `esms.connection.sms.environment` | no | `sandbox` to use the eSMS test environment: nothing is delivered or charged. Unset or `production`: real messages. |

`GET /api/v1/kernel/settings?group=connection` shows which fields are set, when the connection was
last used and its last error.

## 4. Test the connection

```bash
curl -X POST https://your-server/api/v1/kernel/settings/esms.connection.sms.api_key/test \
  -H "Authorization: Bearer $TOKEN"
```

This asks eSMS for the account balance (the balance itself is not returned). `"ok": true` means the
ApiKey and SecretKey are accepted. It does not check the brandname or the templates: for that, set
`esms.connection.sms.environment` to `sandbox`, trigger a code, and read the connection's last error.

eSMS has no separate sandbox address or sandbox account: the test environment is the same API and
the same ApiKey / SecretKey, with a `Sandbox` flag the adapter adds to each request. Set the
setting back to `production` (or remove it) to send real messages.

## Webhook

None. The adapter does not use the eSMS status callback (it carries no signature), so there is no
address to paste into eSMS.

## Errors

| Code | Meaning | Retry |
| --- | --- | --- |
| `provider.not_configured` | this tenant has not set the named setting | after setting it |
| `esms.credentials_rejected` | wrong ApiKey / SecretKey (eSMS 101) | after fixing |
| `esms.insufficient_balance` | not enough balance (eSMS 103) | yes, after topping up |
| `esms.brandname_rejected` | brandname unknown or not active (eSMS 104) | after fixing |
| `esms.template_rejected` | text matches no registered template (eSMS 146) | after registering |
| `esms.carrier_not_registered` | brandname not registered with that mobile network (eSMS 177) | after registering |
| `esms.phone_rejected` | number malformed or blocked (eSMS 99, 108) | no |
| `esms.content_blocked` | text contains a blocked keyword (eSMS 201) | no |
| `esms.ip_not_allowed` | server IP not on the eSMS allow list (eSMS 140) | after eSMS adds it |
| `esms.account_inactive` | account not activated or locked (eSMS 143) | after eSMS unlocks it |
| `esms.request_rejected` | any other eSMS code; the message names it | no |
| `esms.unavailable` | eSMS did not answer or answered with a server error | yes |
| `esms.marketing_unsupported`, `esms.phone_unsupported` | see the table at the top | no |

A message sent again with the same reference is not delivered twice: the reference is passed as the
eSMS `RequestId`, which eSMS refuses to repeat for 24 hours.

Phone numbers and message texts are never written to logs.
