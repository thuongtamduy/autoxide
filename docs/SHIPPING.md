# Shipping (`shipping` module)

[Tiếng Việt](SHIPPING.vi.md)

The `shipping` module keeps buyers' address books, computes shipping rates set by each shop, plugs in
shipping carriers as connector modules, follows shipments and reconciles cash on delivery (COD). It does not
depend on `orders` or `catalog`: orders quote through `autoxide_shipping::api` and send shipment requests as
events.

This guide grows with the module. Sections marked *(coming)* describe parts still being built.

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

## Rates, shipments, COD, delivery dates *(coming)*

Zones by province, weight tiers, free shipping, store pickup, carrier accounts, webhooks, shipments with
parcels, COD reconciliation, delivery calendars and the public estimate are documented here as they land.

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `AUTOXIDE_SHIPPING_FAKE_CARRIER` | `false` | Register the in-memory fake carrier (development only) |
| `AUTOXIDE_SHIPPING_CARRIER_TIMEOUT_SECONDS` | `10` | Longest wait for one carrier call |
| `AUTOXIDE_SHIPPING_MAX_ADDRESSES_PER_BUYER` | `20` | Addresses one buyer may keep |
| `AUTOXIDE_SHIPPING_WEBHOOK_MAX_BYTES` | `262144` | Largest carrier webhook body |
| `AUTOXIDE_SHIPPING_COD_SETTLEMENT_DAYS` | `14` | Days before an unsettled COD amount is overdue |

Carrier credentials, delivery calendars and holidays are data of each tenant, not environment variables.
