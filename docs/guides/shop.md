# Opening a shop: your first order from product to e-invoice

This guide takes a fresh [quickstart](../../README.md#quickstart) server to a first real sale: a
product with a price and stock, a guest buyer's cart, delivery quoted by Giao Hàng Nhanh (GHN),
payment with MoMo, an SMS through eSMS for each step, the parcel booked with GHN, and the e-invoice
issued through SePay eInvoice once the parcel is delivered. Every command is a `curl` call you can
paste. The same flow runs as a test on SQLite and PostgreSQL on every build
(`crates/app/src/e2e/shop.rs`).

Vietnamese: [`shop.vi.md`](shop.vi.md).

> **Experimental connectors.** The four provider adapters (MoMo, GHN, eSMS, SePay eInvoice) were
> written from each provider's public documentation and are not yet verified against the live
> services. Use each provider's sandbox first, then a small real order. Money always goes straight
> from the buyer to **your own** MoMo account; Autoxide never holds it.

## What you need

- A server from the quickstart, reachable from the internet over HTTPS (MoMo and GHN send their
  notifications to it). For a test on your computer, a tunnel (for example `cloudflared` or `ngrok`)
  to `http://127.0.0.1:8080` is enough.
- An account at each provider, in your shop's name: a MoMo business account (Partner Code, Access
  Key, Secret Key), a GHN shop (API token and ShopId), an eSMS account with a registered brandname
  (ApiKey, SecretKey) and a SePay eInvoice account (client id, client secret, provider account id).
  Each guide below says where to find these values.
- Before selling to the public in Vietnam: notify your sales website to the Ministry of Industry and
  Trade (Decree 52/2013/ND-CP as amended) and publish your terms; see the `compliance` module. This
  guide does not replace legal advice.

## 1. Turn on the connectors

The quickstart runs the official modules only. Add these lines to `.env` next to
`compose.quickstart.yml`, then start again:

```bash
cat >> .env <<'EOF'
AUTOXIDE_MODULES=official,momo,ghn,esms,einvoice_sepay
AUTOXIDE_HTTP_PUBLIC_URL=https://shop-api.example.com
EOF
docker compose -f compose.quickstart.yml up -d
```

`AUTOXIDE_HTTP_PUBLIC_URL` is the HTTPS address at which the internet reaches your server (your
tunnel's address when testing). The quickstart already loaded the reference data this guide needs:
Vietnamese administrative units (`address.import`) and the tax pack (`tax.import_pack`).

Sign in and keep the token (it lasts 15 minutes; sign in again when a call answers `401`). Storing
a provider account needs a recent sign-in, so sign in again before step 2 if it has been a while.

```bash
API=http://127.0.0.1:8080
TOKEN=$(curl -sS "$API/api/v1/auth/login" -H 'Content-Type: application/json' \
  -d '{"email":"admin@example.com","password":"<your password>"}' | jq -r .data.access_token)
ORG=$(curl -sS "$API/api/v1/organizations/units?limit=1" -H "Authorization: Bearer $TOKEN" | jq -r '.data[0].id')
```

`ORG` is your shop: the root unit of your organization tree.

## 2. Connect your provider accounts

Every value is stored for your shop only; secrets are encrypted and never shown again. The details of
each provider are in its own guide: [MoMo](providers/momo.md), [GHN](providers/ghn.md),
[eSMS](providers/esms.md), [SePay eInvoice](sepay-einvoice.md).

**MoMo** — payments:

```bash
for pair in partner_code=MOMOXXXX access_key=<Access Key> secret_key=<Secret Key>; do
  curl -sS -X PUT "$API/api/v1/kernel/settings/momo.connection.account.${pair%%=*}" \
    -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
    -d "{\"value\": \"${pair#*=}\"}"
done
```

The page a buyer returns to after paying must be one of your shop's own addresses: an `https://` address on a
domain you verified (`POST /api/v1/kernel/domains`), or an origin you list here — your website, your app's
universal link domain, `http://localhost:3000` while developing. Any other `return_url` is refused, so a payment
link cannot send your buyers to someone else's site:

```bash
curl -sS -X PUT "$API/api/v1/kernel/settings/ledger.payments.return_origins" \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"value": "https://shop.example.com"}'
```

**eSMS** — the SMS sent to buyers. Until you set it, SMS notifications are skipped with the reason
`channel_disabled` (nothing is retried, nothing fails):

```bash
for pair in api_key=<ApiKey> secret_key=<SecretKey> brandname=<Brandname>; do
  curl -sS -X PUT "$API/api/v1/kernel/settings/esms.connection.sms.${pair%%=*}" \
    -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
    -d "{\"value\": \"${pair#*=}\"}"
done
```

**SePay eInvoice** — e-invoices, and an invoice series that uses it:

```bash
for pair in client_id=<client id> client_secret=<client secret> provider_account_id=<provider account id>; do
  curl -sS -X PUT "$API/api/v1/kernel/settings/einvoice_sepay.connection.api.${pair%%=*}" \
    -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
    -d "{\"value\": \"${pair#*=}\"}"
done
curl -sS "$API/api/v1/invoices/series" -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"kind": "vat_invoice", "symbol": "1C26TAA", "year": 2026, "einvoice_provider": "sepay"}'
```

**GHN** — delivery. The carrier account answers with `webhook_path` once: keep it. Then a delivery
method that uses the account and books each parcel with GHN by itself (`auto_book`):

```bash
GHN=$(curl -sS "$API/api/v1/shipping/carrier-accounts" -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' \
  -d "{\"org_id\": \"$ORG\", \"carrier_code\": \"ghn\", \"name\": \"GHN\", \"credentials\": {\"token\": \"<GHN token>\"}, \"settings\": {\"shop_id\": <ShopId>}}")
echo "$GHN" | jq -r .data.webhook_path
curl -sS "$API/api/v1/shipping/methods" -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d "{\"org_id\": \"$ORG\", \"code\": \"ghn\", \"kind\": \"carrier\", \"names\": {\"vi\": \"Giao Hàng Nhanh\", \"en\": \"GHN\"}, \"currency\": \"VND\", \"carrier_account_id\": \"$(echo "$GHN" | jq -r .data.id)\", \"auto_book\": true}"
```

In the GHN dashboard, register `AUTOXIDE_HTTP_PUBLIC_URL` + `webhook_path` as the status webhook (see
the [GHN guide](providers/ghn.md#status-webhook)). MoMo needs nothing: Autoxide sends your
notification address with every payment.

## 3. A product with a price and stock

Give the weight and size of one item: GHN prices the parcel from them.

```bash
PRODUCT=$(curl -sS "$API/api/v1/catalog/products" -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"kind": "physical", "tax_category": "vat.standard", "locale": "vi", "text": {"name": "Ấm trà"},
       "variant": {"sku": "TEA-1", "weight_grams": 600, "length_mm": 300, "width_mm": 200, "height_mm": 100}}')
PRODUCT_ID=$(echo "$PRODUCT" | jq -r .data.id)
VARIANT=$(echo "$PRODUCT" | jq -r '.data.variants[0].id')

LIST=$(curl -sS "$API/api/v1/catalog/price-lists" -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"name": "Retail", "currency": "VND", "tax_inclusive": true}' | jq -r .data.id)
curl -sS "$API/api/v1/catalog/price-lists/$LIST/prices" -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d "{\"reference\": \"opening\", \"prices\": [{\"variant_id\": \"$VARIANT\", \"amount\": {\"amount\": \"150000\", \"currency\": \"VND\"}}]}"

curl -sS "$API/api/v1/inventory/adjustments" -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d "{\"org_id\": \"$ORG\", \"reference\": \"opening-1\", \"lines\": [{\"variant_id\": \"$VARIANT\", \"quantity\": 10, \"reason_code\": \"found\", \"unit_cost\": {\"amount\": \"90000\", \"currency\": \"VND\"}}]}"

curl -sS -X POST "$API/api/v1/catalog/products/$PRODUCT_ID/status" -H "Authorization: Bearer $TOKEN" \
  -H 'If-Match: *' -H 'Content-Type: application/json' -d '{"status": "active"}'
```

## 4. Buy it as a guest

This is what your website does. No sign-in: the cart is identified by the `Cart-Token` it returns.
Prices, discounts, tax and delivery fee are always computed by the server.

```bash
CART=$(curl -sS "$API/api/v1/orders/carts" -H 'Content-Type: application/json' -d '{}')
CART_ID=$(echo "$CART" | jq -r .data.id)
CART_TOKEN=$(echo "$CART" | jq -r .data.cart_token)

curl -sS -X PUT "$API/api/v1/orders/carts/$CART_ID/lines/$VARIANT" -H "Cart-Token: $CART_TOKEN" \
  -H 'Content-Type: application/json' -d '{"quantity": 2}'

# Where to deliver: codes from GET /api/v1/address/provinces and GET /api/v1/address/units?province=<code>.
curl -sS -X PUT "$API/api/v1/orders/carts/$CART_ID/delivery" -H "Cart-Token: $CART_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"recipient_name": "Nguyễn An", "phone": "0912345678",
       "address": {"country": "VN", "province_code": "01", "commune_code": "00004", "line": "12 Tràng Tiền"}}' \
  | jq '.data.shipping.options'
```

The options list GHN's fee for this parcel (the recipient, the address and the parcel's weight and
size are sent to GHN). Choose one by sending its `option_key` with the same address, then place the
order. Keep the `Idempotency-Key`: sending the same request again returns the same order.

```bash
QUOTE=$(curl -sS -X PUT "$API/api/v1/orders/carts/$CART_ID/delivery" -H "Cart-Token: $CART_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"recipient_name": "Nguyễn An", "phone": "0912345678", "shipping_option": "<option_key>",
       "address": {"country": "VN", "province_code": "01", "commune_code": "00004", "line": "12 Tràng Tiền"}}' \
  | jq -r .data.quote_token)

curl -sS "$API/api/v1/orders/checkout" -H "Cart-Token: $CART_TOKEN" -H 'Idempotency-Key: first-order-1' \
  -H 'Content-Type: application/json' \
  -d "{\"cart_id\": \"$CART_ID\", \"quote_token\": \"$QUOTE\", \"payment_method\": \"wallet\",
       \"return_url\": \"https://shop.example.com/thank-you\",
       \"contact\": {\"name\": \"Nguyễn An\", \"phone\": \"0912345678\"}}"
```

`return_url` is the page of your website the buyer comes back to after paying (on one of your shop's
addresses, see step 2); MoMo requires one (or set `momo.connection.account.return_url` once for your shop). The answer has the order
(`pending_payment`) and `payment_actions[0]`: a `redirect` to MoMo's payment page. When the buyer has paid, MoMo notifies your server, and the order becomes `confirmed` / `paid` on
its own. The buyer receives an SMS for the order placed and for the payment confirmed.

## 5. Ship it

Note the order id from the checkout answer. Requesting the shipment accepts the order (`preparing`);
GHN books the parcel by itself because the method has `auto_book`:

```bash
ORDER=<order id>
curl -sS "$API/api/v1/orders/$ORDER/shipments" -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' -d '{}'
curl -sS "$API/api/v1/shipping/shipments?order_id=$ORDER" -H "Authorization: Bearer $TOKEN"
```

From then on GHN's status webhook moves the order: picked up → `shipping`, delivered → `delivered`.
At delivery the e-invoice is issued through SePay eInvoice (`orders.invoice_timing` is
`on_delivered` by default) and the buyer receives an SMS.

```bash
curl -sS "$API/api/v1/orders/$ORDER" -H "Authorization: Bearer $TOKEN" | jq '.data | {status, payment_status}'
curl -sS "$API/api/v1/invoices/documents?source_module=orders&source_id=$ORDER" -H "Authorization: Bearer $TOKEN" \
  | jq '.data[0] | {status, number, authority_code}'
```

## When something does not happen

| What you see | Why, and what to do |
| --- | --- |
| A call answers `provider.not_configured` | A field of that provider account is not set. `GET /api/v1/kernel/settings?group=connection` lists what each connection needs. |
| An SMS has status `skipped`, reason `channel_disabled` (`GET /api/v1/notifications/messages`) | eSMS is not set up for your shop yet. Messages before that are not sent again. |
| Checkout answers that `provider` is required | The server has several payment partners. Set your default one: `PUT /api/v1/kernel/settings/ledger.payments.default_provider` with `{"value": "momo"}`. |
| `payment_actions[0].kind` is `none` | The payment could not be started at MoMo; `payment_actions[0].error` says why (`code` and `message`, e.g. `momo.return_url_required`). Fix it, then start the payment again: `POST /api/v1/orders/{id}/payment-link` (shop staff), `POST /api/v1/orders/me/orders/{id}/payment-link` (a signed-in buyer) or `POST /api/v1/orders/public/orders/{id}/payment-link` with the `Order-Token` header (a guest buyer); the buyer may send `{"return_url": "…"}`. |
| `payment_actions[0].error.code` is `orders.return_url_not_allowed` (or `payment-link` answers 422 with that code) | `return_url` is not on one of your shop's addresses. Add its origin to `ledger.payments.return_origins` (step 2) or verify the domain, then start the payment again. |
| `GET /api/v1/ledger/payments/{id}` answers 404 for a `payment_id` | `payment_actions[].payment_id` is the order's own payment. Use `payment_actions[].ledger_payment_id` with the `ledger` endpoints. |
| `payment-link` returns a new `payment_id` | The partner reported the previous collection failed (the buyer declined or gave up), so a new attempt was started (status `superseded` on the old one). Show the buyer the new action and use its `status_token`. If the partner still collects the old one, that money is refunded through the partner on its own. |
| A payment is `needs_review` and the order stays `pending_payment` | The partner reported another amount than the amount due. A corrected notification for the same transaction does not clear it: someone with `ledger.reconcile` finds the `amount_mismatch` item (`GET /api/v1/ledger/exceptions?status=open`) and resolves it with `POST /api/v1/ledger/exceptions/{id}/resolve`: `{"action": "accept_received", "payment_id": "<ledger payment id>"}` records the amount collected and confirms the order if it is still waiting for payment. Resolve it before the order's payment deadline (30 minutes by default, `AUTOXIDE_LEDGER_PAYMENT_EXPIRY_MINUTES`): after it, the order is already cancelled, so the amount collected is refunded through the partner on its own and the order is not reopened. `refunded_outside` or `ignore` close the item and the order is cancelled at its deadline. |
| A `late_payment` item is open in `GET /api/v1/ledger/exceptions` | Money arrived after the payment's deadline or after the partner reported it failed. The order decides: if it still counts the money (the order goes on), the item closes on its own (resolution `kept`) within moments; when the order no longer needs it, Autoxide refunds it through the partner on its own, and the item closes once the payment is refunded in full (resolution `refunded`). Never refund an item whose order went on. An item still open means the partner refused that refund: first check `refunds` in `GET /api/v1/ledger/payments/{id}` (a refund may still be pending at the partner), and only then refund outside Autoxide and resolve the item with `refunded_outside`, so the buyer is not refunded twice. |
| A `refund_failed` item is open in `GET /api/v1/ledger/exceptions` | The partner refused a refund of that payment, or the refund could not be sent at all (for example a bank transfer, which no partner refunds, of an order cancelled after the money arrived). Nothing is sent again on its own. Refund the buyer outside Autoxide, then resolve the item with `refunded_outside`; if a later refund through the partner returns the whole payment, the item closes on its own. A refund answered with `409` or `429`, or with a server error, was not refused: it stays `pending` and is sent again with the same reference. |
| The order stays `pending_payment` | MoMo could not reach `AUTOXIDE_HTTP_PUBLIC_URL`. Check the address and look at `GET /api/v1/kernel/settings?group=connection` for the last error. |
| GHN's option is missing from the cart | GHN refused the quote. The carrier account shows the last error (`GET /api/v1/shipping/carrier-accounts`); check the item's weight and the address. |
| The order does not move after GHN picked the parcel | The GHN webhook is not registered, or GHN has not applied it yet (about 15 minutes). |
| An `ETag` / `If-Match` error | Read the record again and send its current `ETag` (`If-Match: *` skips the check). |
