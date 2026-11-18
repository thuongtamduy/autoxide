# Shipping (`shipping` module)

[Tiếng Việt](SHIPPING.vi.md)

The `shipping` module keeps buyers' address books, computes shipping rates set by each shop, plugs in
shipping carriers as connector modules, follows shipments and reconciles cash on delivery (COD). It does not
depend on `orders` or `catalog`: orders quote through `autoxide_shipping::api` and send shipment requests as
events.

## Address book

Signed-in buyers keep up to `AUTOXIDE_SHIPPING_MAX_ADDRESSES_PER_BUYER` (default 20) addresses:

| Method | Path | Who |
|---|---|---|
| `GET`, `POST` | `/api/v1/shipping/me/addresses` | any signed-in account, own addresses only |
| `GET`, `PATCH`, `DELETE` | `/api/v1/shipping/me/addresses/{id}` | same |
| `POST` | `/api/v1/shipping/me/addresses/{id}/default` | same |
| `GET` | `/api/v1/shipping/buyers/{buyer_id}/addresses` | `shipping.addresses.read` (customer care) |

- **Vietnam:** `province_code` and `commune_code` come from the official list of administrative units of the
  `address` module (two levels since 2025-07-01). The commune must belong to the province and be in force; the
  names are saved with the address, so a later change of the list does not change saved addresses.
- **Other countries:** a generic form — `province_code` is the region (state, province…), `city` the city,
  `postal_code` optional. In responses the city is `address.commune_name` and `address.commune_code` is empty.
- Exactly one address is the default (the first one automatically). Deleting the default makes the most
  recently changed other address the default.
- An address book is personal data: it is part of the account's data export and is erased when the account is
  deleted. Shipments keep their own copy of the recipient.

Other modules read it through `dyn AddressBook` (`get`, `default_for`).

## Shipping rates

Each shop sets its own rates under `/api/v1/shipping` (permission `shipping.settings.manage` at the unit):

| Method | Path | What |
|---|---|---|
| `GET`, `POST` | `/zones` | zones: groups of provinces (or whole countries) |
| `GET`, `PATCH`, `DELETE` | `/zones/{id}` | one zone (deleting it removes its tiers) |
| `PUT` | `/zones/{id}/regions` | replace the provinces of a zone |
| `GET`, `POST` | `/methods` | shipping methods: `table_rate`, `carrier`, `pickup` |
| `GET`, `PATCH`, `DELETE` | `/methods/{id}` | one method with its tiers and pickup points |
| `PUT` | `/methods/{id}/rates` | replace the weight tiers of a method |
| `PUT` | `/methods/{id}/pickup-points` | replace the stores of a `pickup` method |
| `POST` | `/quote` | quote a cart (`shipping.read` at the store) — the same computation orders use |

- **Inheritance:** a store uses the active methods of its own unit and of every unit above it; a method of a
  nearer unit replaces one with the same `code` higher up (a store can override the chain's `standard`).
- **Zones:** a zone listing the destination province wins over a zone for the whole country.
- **Weight:** chargeable weight = max(actual weight, volumetric weight); volumetric weight of an item =
  length × width × height (mm³) / `volumetric_divisor` (default 6000) grams, rounded up. Items that do not need
  shipping (digital goods, services) weigh nothing; a cart with none to ship gets no options. An item without
  weight uses the method's `default_item_weight_grams`; without one, the method is listed in `unavailable`
  with `shipping.weight_missing`.
- **Tiers:** `min_weight_grams ≤ weight < max_weight_grams` (no maximum = no limit) and longest side ≤
  `max_side_mm`; each `step_grams` (or part) above the minimum adds `step_amount`. Overlapping tiers are refused.
- **Free shipping:** `free_over` compared with the order value after discounts; the option shows `free_over` so
  a cart can say how much more to buy.
- **VAT:** fees are VAT-inclusive; each option carries the method's `tax_category` so the order puts the VAT
  line of the shipping fee on the invoice.
- **Store pickup:** one option per store, fee = the method's `handling_fee`, no destination or weight needed.
- Every option has an `option_key`. Send it back when placing the order: `ShippingRates::confirm_option`
  answers `shipping.option_changed` when the fee or destination changed.

## Carrier accounts

`/api/v1/shipping/carrier-accounts` (permission `shipping.carriers.manage`, plan feature `shipping.carriers`):
`GET`, `POST`, `PATCH /{id}`, `POST /{id}/test`. Credentials are stored encrypted per tenant and never shown again
(only `credential_fields`). The create answer holds `webhook_path` once — give it to the carrier. `check_status`
(`never`, `ok`, `failed`) is set by the test call and whenever the carrier refuses the credentials. A `carrier`
method quotes through its account; a carrier that does not answer within `AUTOXIDE_SHIPPING_CARRIER_TIMEOUT_SECONDS`
is listed in `unavailable` (`shipping.carrier_unavailable`) without breaking the quote.

## Shipments

| Method | Path | Permission |
|---|---|---|
| `GET` | `/shipments?order_id&status&org_id&tracking`, `/shipments/{id}` | `shipping.shipments.read` |
| `POST` | `/shipments` (by hand), `/shipments/{id}/book`, `/cancel`, `/split`, `/status` | `shipping.shipments.manage` |
| `PUT` | `/shipments/{id}/parcels` | `shipping.shipments.manage` |
| `GET` | `/shipments/{id}/label` | `shipping.shipments.read` |

- Orders ask for a shipment with the event `orders.fulfillment_requested` (returns:
  `orders.return_pickup_requested`), payload `{"after": {"shipment": ShipmentRequest}}`. The same `order_id` +
  `request_key` never creates a second shipment; the same key with other content is refused and logged.
  `orders.order_cancelled` cancels the shipments not picked up yet.
- Statuses: `pending`, `booking`, `ready_to_ship`, `picked_up`, `in_transit`, `out_for_delivery`, `delivered`,
  `delivery_failed`, `returning`, `returned`, `ready_for_pickup`, `picked_up_by_buyer`, `cancelled`, `lost`. They only
  move forward along a fixed table (a new attempt after `delivery_failed` is allowed). Carrier updates that arrive
  late or out of order are kept in the tracking history but do not move the status back.
- Booking calls the carrier outside any database transaction: `pending → booking` is saved first, then the carrier
  is called with the shipment id as `client_reference`. A shipment stuck in `booking` asks the carrier
  (`find_by_reference`) before ever booking again (every 5 minutes for shipments stuck over 10 minutes).
- A method with `auto_book` books as soon as the order asks; otherwise staff check the goods and press book.
- Parcels: one parcel at first; `PUT /parcels` splits the items into several parcels before booking. `split` moves
  some lines to a new shipment (partial delivery); cash on delivery stays with the original.
- Events (`after` is a `ShipmentEvent`, without personal data): `shipping.shipment_created`, `…_booked`,
  `…_status_changed`, `…_delivered`, `…_delivery_failed`, `…_returned`, `…_cancelled`, `shipping.booking_failed`,
  `shipping.cod_collected`.

## Carrier webhooks

`POST /api/v1/shipping/webhooks/{carrier_code}/{token}` is public. The connector checks the signature on the raw
body; a wrong signature answers 401 and nothing is stored. A valid body is stored as received (secret headers
removed) and answered 202; a job applies it. Repeated updates are applied once (`event_key`); bodies above
`AUTOXIDE_SHIPPING_WEBHOOK_MAX_BYTES` answer 413. Carriers without webhooks are polled every 30 minutes.

## Cash on delivery (COD)

The carrier collects the cash and pays it to the seller under its contract; Autoxide only records who owes what and
reconciles (it never holds the money). With the `ledger` module installed:

- a delivered COD shipment records "the carrier owes the amount" (`clearing.carrier_cod`) against the buyer's
  receivable;
- `POST /api/v1/shipping/cod/statements` (permission `shipping.cod.manage`) imports a carrier statement: CSV with the
  columns `tracking_number`, `cod_amount`, `fee` (minor units) or `fetch: true` for carriers with a statement API.
  Matching lines settle the shipment (money in transit + carrier fee against what the carrier owed; event
  `shipping.cod_settled` with `seller_org_id`). The same reference with the same content is not imported twice.
- Lines that differ (amount, unknown tracking number, not delivered) and COD still missing from statements after
  `AUTOXIDE_SHIPPING_COD_SETTLEMENT_DAYS` become discrepancies (`GET /cod/discrepancies`, event
  `shipping.cod_mismatch`). Resolve one with a code of the reason list `shipping.cod_resolution` and a note;
  `written_off` writes the missing amount off. `refresh-tracking` asks the carrier for the latest tracking first.

## Contract for other modules

```rust
let rates = context.require::<dyn autoxide_shipping::api::ShippingRates>()?;   // quote, confirm_option
let book = context.require::<dyn autoxide_shipping::api::AddressBook>()?;
let carriers = context.require::<dyn autoxide_shipping::api::CarrierRegistry>()?;
```

Register your module after `shipping` in `crates/app/src/modules.rs`. Build requests with the constructors
(`QuoteRequest::new`, `QuoteItem::new(…).weight(…)`, `ShipmentRequest::new`): the types are `#[non_exhaustive]`
so new fields never break callers.

`orders` asks for a shipment by publishing `orders.fulfillment_requested` (or `orders.return_pickup_requested`)
with `{"after": {"shipment": <ShipmentRequest>}}`; `shipping` answers with `shipping.shipment_*` events whose
`after` is a `ShipmentEvent` (no personal data).

## Writing a carrier connector

A connector is its own module (`shipping-<carrier>`) registered after `shipping`. It implements
`ShippingCarrier` and registers it in `Module::start`:

```rust
fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
    let registry = context.require::<dyn autoxide_shipping::api::CarrierRegistry>()?;
    registry.register(Arc::new(MyCarrier::new()))
}
```

Rules every connector follows (see `autoxide_shipping::api::fake::FakeCarrier` for a complete example):

- `create_shipment` must not create a second shipment for the same `client_reference` (the Autoxide shipment
  id); `find_by_reference` finds one created by a call whose answer was lost.
- `parse_webhook` checks the signature on the raw body (constant-time comparison) before reading it, and returns
  `shipping.webhook_signature` when it is wrong.
- Convert carrier times to UTC (Vietnamese carriers often send local time without a zone).
- Map carrier statuses to the common `ShipmentStatus`; keep the carrier's own text in `carrier_status`.
- Never log credentials: `CarrierCredentials` hides its values from `Debug`.

`AUTOXIDE_SHIPPING_FAKE_CARRIER=true` registers the fake carrier `fake` for trying everything on a development
machine. `autoxide doctor` warns while it is on; never turn it on in production.

## Delivery dates

Delivery calendars live under `/api/v1/shipping/calendars` (`shipping.settings.manage`): a time zone (default
`Asia/Ho_Chi_Minh`), working days of the week, and special days set with `PUT /calendars/{id}/days/{YYYY-MM-DD}` —
`closed` (holiday, entered when it is announced; nothing is hard-coded) or `special_hours` with its own cut-off. A
method links a calendar (`calendar_id`), a daily cut-off (`cutoff_minute`, 660 = 11:00) and `same_day`.

Every option of a quote carries `eta`: `earliest_date` and `latest_date` as calendar dates in the calendar's time
zone (working days after the dispatch day), and `order_by`, today's cut-off when it is still ahead. Methods without a
calendar use Monday to Saturday, Vietnam time. Store pickup options carry `ready_at` (order time + the pickup point's
`prep_hours`, moved to the next opening time of the store from its public profile, 08:00–21:00 when the store has
none) and `hold_days`.

## Public estimate for product pages

`GET /api/v1/shipping/public/estimate?variant_id=…&quantity=1&province_code=79` needs no sign-in. It runs the same
quote as checkout for a one-line cart to a province (fees, free-shipping threshold, `eta` with today's cut-off) and
answers `estimate: true` — the real fee is always computed again for the cart. The variant's weight, size, selling
unit and price come from the `catalog` module. Answers are cacheable for 60 seconds.

## MCP tools

`shipping.quote` (read only), `shipping.list_shipments`, `shipping.get_shipment`, `shipping.book_shipment`,
`shipping.cancel_shipment` (destructive), `shipping.update_status`, `shipping.list_cod_discrepancies` — with the
same permissions as the matching REST endpoints.

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `AUTOXIDE_SHIPPING_FAKE_CARRIER` | `false` | Register the in-memory fake carrier (development only) |
| `AUTOXIDE_SHIPPING_CARRIER_TIMEOUT_SECONDS` | `10` | Longest wait for one carrier call |
| `AUTOXIDE_SHIPPING_MAX_ADDRESSES_PER_BUYER` | `20` | Addresses one buyer may keep |
| `AUTOXIDE_SHIPPING_WEBHOOK_MAX_BYTES` | `262144` | Largest carrier webhook body |
| `AUTOXIDE_SHIPPING_COD_SETTLEMENT_DAYS` | `14` | Days before an unsettled COD amount is overdue |

Carrier credentials, delivery calendars and holidays are data of each tenant, not environment variables.
