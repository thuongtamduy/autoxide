# Taking payments with MoMo

[MoMo](https://momo.vn) is an e-wallet in Vietnam. With the `momo` adapter, a payment created in Autoxide
gets a MoMo payment page (and a QR code) for the buyer, and is confirmed by itself when MoMo reports the
result. The money goes from the buyer to **your own** MoMo business account; Autoxide never holds it.

Vietnamese: [`momo.vi.md`](momo.vi.md).

> **Experimental.** This adapter was written from the provider's public documentation; not yet verified
> against the live service. Try it with MoMo's sandbox, then with a small real payment, before relying on
> it. It uses MoMo's Payment Gateway API v2 (`captureWallet`).

Each tenant connects **its own** MoMo account. Nothing here is a server-wide account, and the keys you
store are encrypted and never shown again.

## Before you start

- The server operator turns the module on: `AUTOXIDE_MODULES=official,momo` (see
  [`docs/MODULES.md`](../../MODULES.md)).
- The server operator sets `AUTOXIDE_HTTP_PUBLIC_URL` to the address at which the internet reaches the
  API (`https://…`). MoMo sends payment results to that address.
- You have a MoMo business account. In the MoMo for Business portal (business.momo.vn), the integration
  page shows three values: **Partner Code**, **Access Key** and **Secret Key**. The sandbox and the
  production environment each have their own set.

## 1. Store your MoMo account

You need the permission `kernel.settings.manage`.

```bash
curl -X PUT "$API/api/v1/kernel/settings/momo.connection.account.partner_code" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"value": "<Partner Code>"}'

curl -X PUT "$API/api/v1/kernel/settings/momo.connection.account.access_key" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"value": "<Access Key>"}'

curl -X PUT "$API/api/v1/kernel/settings/momo.connection.account.secret_key" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"value": "<Secret Key>"}'

# Test it: Autoxide asks MoMo about a payment that does not exist. MoMo answers "not found" when it
# accepts your keys.
curl -X POST "$API/api/v1/kernel/settings/momo.connection.account.secret_key/test" \
  -H "Authorization: Bearer $TOKEN"
```

| Setting | Required | What it is |
| --- | --- | --- |
| `momo.connection.account.partner_code` | yes | Partner Code of your MoMo business account. |
| `momo.connection.account.access_key` | yes | Access Key. Stored encrypted, never returned. |
| `momo.connection.account.secret_key` | yes | Secret Key: signs every request and checks every notification. Stored encrypted, never returned. |
| `momo.connection.account.environment` | no | `production` (default) or `sandbox`. Use `sandbox` with MoMo's test keys: no real money moves. |
| `momo.connection.account.return_url` | no | Page of your website the buyer returns to after paying, used when a payment has no return URL of its own. |

The two keys are secrets: set them over REST as above. The MCP tool `kernel.set_setting` refuses
secrets, so a key never passes through an AI agent.

## 2. The notification address — nothing to paste

MoMo calls this "IPN". Autoxide sends your own notification address to MoMo with every payment it
creates, so there is nothing to configure at MoMo. You can see the address (its last part identifies
your tenant and cannot be guessed):

```bash
curl "$API/api/v1/kernel/connections/webhooks" -H "Authorization: Bearer $TOKEN"
```

Look for the line `momo.connection.account`. Every notification is checked with **your** Secret Key
before anything is recorded; a notification signed with another key is refused with
`payment.signature_invalid`. MoMo may send the same notification several times; it is recorded once.

## 3. Take a payment

Create a payment with the partner `momo` and the method `wallet`, for example with
`POST /api/v1/ledger/payments` (permission `ledger.payments_create`), or through a module that takes
payments for you (orders, bookings…). The answer has:

- `checkout_url` — MoMo's payment page. Send the buyer there.
- `qr_payload` — the QR code content, when MoMo returns one for your account.

When the buyer has paid, MoMo notifies Autoxide and the payment becomes `succeeded`. If MoMo reports a
different amount than the payment asked for, nothing is booked: the payment becomes `needs_review` and
waits for a person in the reconciliation queue.

Limits from MoMo's documentation: VND only, 1,000 to 50,000,000 VND per wallet payment.

## 4. Refunds

Refund a payment as usual (`POST /api/v1/ledger/payments/{id}/refunds`). Full and partial refunds are
sent to MoMo and the answer is final at once.

## Error codes

| Code | Meaning | What to do |
| --- | --- | --- |
| `provider.not_configured` | A required setting of step 1 is missing. | Set it; the message names it. |
| `momo.account_rejected` | MoMo refused your keys (its result codes 11 and 13). | Check the three values and the environment. |
| `momo.return_url_required` | MoMo needs a return page. | Send `return_url` with the payment, or set `momo.connection.account.return_url`. |
| `momo.amount_out_of_range` | Outside 1,000–50,000,000 VND. | Use another payment method. |
| `momo.currency_not_supported` | Not VND. | Charge in VND. |
| `momo.order_already_created` | An earlier attempt already reached MoMo, and MoMo does not hand out the payment link again. | Create a new payment with a new reference. |
| `momo.refund_already_sent` | An earlier attempt of this refund already reached MoMo; it was not sent again. | Check the refund in the MoMo for Business portal. |
| `payment.provider_rejected` | MoMo refused; the message has MoMo's result code. | See MoMo's list of result codes. |
| `payment.provider_unavailable` | MoMo did not answer, or is busy. | Try again later. |
| `public_url_missing` | The server has no public address. | The operator sets `AUTOXIDE_HTTP_PUBLIC_URL`. |

## What is not covered yet

- Other MoMo products: ATM and credit cards through MoMo, pay later, recurring payments, payouts.
- Statements: MoMo's payment API has none, so reconciliation by statement
  (`POST /api/v1/ledger/reconciliations`) answers `payment.not_supported` for `momo`. Compare with the
  reports of the MoMo for Business portal.

## For operators

| Variable | Default | What it is |
| --- | --- | --- |
| `AUTOXIDE_MOMO_API_URL` | `https://payment.momo.vn` | MoMo's production API. |
| `AUTOXIDE_MOMO_SANDBOX_API_URL` | `https://test-payment.momo.vn` | MoMo's sandbox API, for tenants with `environment` = `sandbox`. |

Sources: MoMo's developer site, <https://developers.momo.vn/v3/docs/payment/api/wallet/onetime/>, and
MoMo's official samples, <https://github.com/momo-wallet/payment> (read on 2026-11-21).
