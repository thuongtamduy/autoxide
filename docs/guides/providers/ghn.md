# Giao Hang Nhanh (GHN)

Connect a tenant's own Giao Hang Nhanh account to Autoxide shipping: shipping fees at checkout, bookings, labels,
cancellation and delivery status.

Vietnamese: [`ghn.vi.md`](ghn.vi.md).

> **Experimental.** This connector is written from GHN's public documentation (<https://developer.ghn.vn>, read on
> 2026-11-21); it is not yet verified against the live service. Try it with a GHN staging account first. The points
> that still need a real account are listed under [Not verified yet](#not-verified-yet).

## For the person who runs the server

The module is off by default. Turn it on by name; `shipping` comes with it:

```bash
AUTOXIDE_MODULES=official,ghn
```

| Setting | Default | Meaning |
| --- | --- | --- |
| `AUTOXIDE_GHN_API_URL` | `https://online-gateway.ghn.vn` | GHN API for live accounts |
| `AUTOXIDE_GHN_SANDBOX_API_URL` | `https://dev-online-gateway.ghn.vn` | GHN staging API, for accounts marked `sandbox` |

There is no setting for a GHN token: every tenant uses its own account. If your GHN token has an allowed-IP list,
add the public IP of this server to it.

## What the tenant needs from GHN

1. Sign in at <https://developer.ghn.vn> (staging: <https://developer.ghn.dev>) with the phone number of the GHN
   account.
2. Account menu → **Manage token**: reveal the **Token API** (GHN sends an OTP to the phone) and copy it.
3. Note the **ShopId** of the shop that ships the parcels (shown in the account information). Parcels are picked up
   at the address of that shop.

Staging and production are separate accounts with separate tokens.

## Connect the account

Needs the permission `shipping.carriers.manage`. The token is stored encrypted and is never returned.

```http
POST /api/v1/shipping/carrier-accounts
{
  "carrier_code": "ghn",
  "name": "GHN main shop",
  "credentials": { "token": "<Token API>" },
  "settings": { "shop_id": 196560 },
  "sandbox": false
}
```

`sandbox: true` is for a token from <https://developer.ghn.dev>. Optional `settings`:

| Setting | Default | Meaning |
| --- | --- | --- |
| `required_note` | `KHONGCHOXEMHANG` | Delivery instruction: `KHONGCHOXEMHANG` (do not open), `CHOXEMHANGKHONGTHU` (may look), `CHOTHUHANG` (may try) |
| `payment_type_id` | `1` | Who pays GHN's fee: `1` the shop, `2` the recipient |
| `label_size` | `a5` | Label page: `a5`, `80x80` or `52x70` |
| `content` | `Hàng hóa` | Description of the goods sent to GHN |

The answer holds `webhook_path` **once** — keep it for the next step.

Then test the connection: `POST /api/v1/shipping/carrier-accounts/{id}/test`. `check_status` becomes `ok`, or
`failed` with `check_error_code`:

| Code | What to do |
| --- | --- |
| `shipping.carrier_unauthorized` | GHN refused the token (wrong token, wrong environment, or this server's IP is not in the allowed list) |
| `ghn.shop_not_found` | The token does not manage this ShopId, or the shop is locked |
| `ghn.account_incomplete` | `token` or `shop_id` is missing |
| `shipping.carrier_unavailable` | GHN did not answer; try again |

Finally create a shipping method of kind `carrier` with this `carrier_account_id` (see
[`docs/SHIPPING.md`](../../SHIPPING.md)). `service_code` is `2` (light goods, under 20 kg), `5` (heavy goods or
several parcels), or empty to choose by weight and number of parcels.

## Status webhook

1. Build the full address: the public address of your Autoxide server + `webhook_path`, for example
   `https://api.example.com/api/v1/shipping/webhooks/ghn/<hook>/<token>`.
2. In the GHN Developer Portal: account menu → **Webhook configuration** → tab **Order** → paste the address →
   **Create webhook**. GHN needs about 15 minutes to apply a change.

Treat the address as a secret: GHN does not sign its callbacks, so the address is what proves a callback belongs to
this account. Autoxide also checks that the `ShopID` in the callback is the shop of the account, and answers 401
otherwise. If the address leaks, create a new carrier account (new address) and turn the old one off.

Repeated callbacks are applied once. Only status changes are used; fee, weight and COD-transfer callbacks are
accepted and ignored.

## How GHN statuses appear

| GHN | Autoxide |
| --- | --- |
| `ready_to_pick`, `picking`, `money_collect_picking` | `ready_to_ship` |
| `picked` | `picked_up` |
| `storing`, `sorting`, `transporting` | `in_transit` |
| `delivering`, `money_collect_delivering` | `out_for_delivery` |
| `delivered` | `delivered` |
| `delivery_fail` | `delivery_failed` |
| `waiting_to_return`, `return`, `return_transporting`, `return_sorting`, `returning`, `return_fail` | `returning` |
| `returned` | `returned` |
| `cancel` | `cancelled` |
| `lost`, `damage`, `scrap` | `lost` |
| `exception` | not applied (handle it with GHN) |

The original GHN status is kept as `carrier_status` on each tracking event.

## Limits

- **Addresses** are sent in Vietnam's two-level model in force since 2025-07-01 (province + ward/commune, by name).
  The old district-based codes of GHN are not used, so there is no code table to maintain.
- Vietnam only, amounts in VND only.
- GHN limits: 50,000 g per order, 200 cm per side, cash on delivery up to 50,000,000 VND. A declared value above
  5,000,000 VND is sent as 5,000,000 (GHN's maximum for insurance).
- A parcel without dimensions is sent as 10 × 10 × 10 cm; GHN re-weighs and may change the fee.
- **Cash on delivery**: GHN collects and pays the shop directly; the money never passes through Autoxide. GHN has no
  statement API in its public documentation, so reconcile by importing GHN's statement
  (`POST /api/v1/shipping/cod/statements`).
- Pickups of returned goods from a buyer are not booked through the connector (`ghn.direction_unsupported`).
- Tracking by polling returns the current status only; the full journey comes from the webhook.

## Error codes

| Code | Meaning |
| --- | --- |
| `ghn.account_incomplete` | The carrier account has no `token` or `shop_id` |
| `ghn.setting_invalid` | An optional setting of the account has a value GHN does not accept |
| `ghn.shop_not_found` | The token does not manage the shop, or the shop is locked |
| `ghn.permission_missing` | GHN answered 403: the account lacks a permission; contact GHN |
| `ghn.currency_unsupported` | The amount is not in VND |
| `ghn.country_unsupported` | The address is outside Vietnam |
| `ghn.service_unknown` | `service_code` is not `2` or `5` |
| `ghn.direction_unsupported` | Return pickups are not supported |
| `ghn.phone_invalid` | GHN refused a phone number |
| `ghn.address_not_recognized` | GHN does not recognise the province or ward name |
| `ghn.route_not_supported` | GHN does not serve the route with the chosen service |
| `ghn.order_not_found` | GHN has no such order in the account |
| `ghn.webhook_body` | A callback body is not GHN's JSON |
| `shipping.carrier_error` | Any other refusal; `message` carries GHN's reason |

## Not verified yet

These follow the documentation where it is not explicit, and need a run with a real account:

- Quotes use GHN's order preview with a placeholder recipient (`Quote`, `0900000000`).
- Province and ward names are sent as the `address` module writes them (for example "Thành phố Hồ Chí Minh");
  GHN's catalogue lists short canonical names plus variants.
- The label page is passed through as GHN serves it (PDF when the file is a PDF, otherwise HTML).
- GHN documents no rate limit.
