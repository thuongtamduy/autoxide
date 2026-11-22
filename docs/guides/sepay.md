# Confirming bank transfers with SePay

[SePay](https://sepay.vn) reports the transactions of your own bank account. With the `sepay` adapter, a
VietQR payment created in Autoxide is confirmed by itself when the buyer's transfer arrives: the money
goes straight to your bank account, and Autoxide only reads the report.

Vietnamese: [`sepay.vi.md`](sepay.vi.md).

> **Experimental.** This adapter was written from the provider's public documentation; not yet verified
> against the live service. Try it with a small real transfer (or SePay's test mode) before relying on it.

The operator turns the adapter on by naming it: `AUTOXIDE_MODULES=official,sepay` (experimental modules
are off by default). Each tenant connects **its own** SePay account. Nothing here is a server-wide setting, and the secrets you
store are encrypted and never shown again.

## 1. Store your SePay account

You need the permission `kernel.settings.manage`.

```bash
# 1. The webhook secret you chose at SePay (HMAC-SHA256)
curl -X PUT "$API/api/v1/kernel/settings/sepay.connection.bank.webhook_secret" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"value": "<secret key of the SePay webhook>"}'

# 2. The API token, to read transactions a webhook missed and to test the connection
curl -X PUT "$API/api/v1/kernel/settings/sepay.connection.bank.api_token" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"value": "<API token of the SePay company>"}'

# 3. Test it
curl -X POST "$API/api/v1/kernel/settings/sepay.connection.bank.api_token/test" \
  -H "Authorization: Bearer $TOKEN"
```

| Setting | What it is |
| --- | --- |
| `sepay.connection.bank.webhook_secret` | Secret Key of the SePay webhook with **HMAC-SHA256** authentication (recommended). |
| `sepay.connection.bank.webhook_api_key` | API Key of the SePay webhook with **API Key** authentication. Used only when no secret key is set. |
| `sepay.connection.bank.api_token` | API token of your SePay company (Company settings > API Keys). |

Notifications are refused until one of the two webhook settings is set. Autoxide never accepts a
notification without authentication, so do not choose "no authentication" at SePay.

## 2. Create the webhook at SePay

Ask Autoxide for your webhook address. It is yours alone; its last part identifies your tenant.

```bash
curl "$API/api/v1/kernel/connections/webhooks" -H "Authorization: Bearer $TOKEN"
```

At my.sepay.vn > Webhooks, add a webhook with:

- **URL**: the `url` of the line `sepay.connection.bank` (or the server's public address followed by
  `path`).
- **Data format**: JSON.
- **Authentication**: HMAC-SHA256, with the same secret key you stored in step 1.
- **Resend when the server answers with an error**: on, so a notification refused during maintenance is
  sent again.

Autoxide answers `{"success": true}` as SePay expects, checks the signature on the raw body, refuses a
notification signed more than 5 minutes ago, and stores a notification that is sent again only once.

## 3. Say which bank account SePay reports

```bash
curl -X POST "$API/api/v1/ledger/bank-accounts" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"bank_bin": "970436", "account_number": "1017588888", "account_name": "CUA HANG A", "feed_provider": "sepay"}'
```

From then on, create payments with `POST /api/v1/ledger/payments/vietqr`. A transfer whose description
keeps the payment's `transfer_code` and whose amount is exact marks the payment as paid. Another amount,
an unknown code or a second transfer waits for a person in `GET /api/v1/ledger/exceptions`.

## Good to know

- SePay sends no webhook for transactions made with a static NAPAS QR code. With the API token set,
  Autoxide reads the transaction list instead when the bank account is polled.
- A transaction that reaches Autoxide by webhook and again from the transaction list is stored once when
  the bank gave it a reference code. Without a reference code the second copy is not matched to a payment
  twice: it waits in the exceptions list.
- The SePay API allows 3 requests per second; when SePay answers "too many requests" the work is retried
  later.
- The checkout page of the SePay payment gateway (cards, NAPAS) is not connected by this adapter.
