# Promotions

[Tiếng Việt](PROMOTIONS.vi.md)

The `promotions` module runs discounts as **rules**. A promotion reaches buyers in one of three ways:
automatically, by **coupon** (a code anyone holding it can use) or as a **voucher** (a one-time right in
one account's wallet). Orders compute a cart, hold the usage of the promotions when the order is placed,
confirm it when paid and release it when cancelled, through `autoxide_promotions::api::Promotions`.
Everything is under `/api/v1/promotions`; every endpoint is described in `docs/openapi.json`, and the
main operations are MCP tools (`promotions.*`).

## Promotions

- `POST /promotions` creates a **draft**. Every promotion has a start and a required end (`ends_at`).
  `trigger` is `automatic`, `code` or `voucher`; `scope` is `items`, `order` or `shipping`, which gives
  the stacking class `product`, `order` or `shipping`.
- Kinds: `percent_off`, `amount_off` (per item for `items`, per order for `order`), `fixed_price`,
  `buy_x_get_y` (`reward_selection`: `cheapest`, `same_item`, `specified`; `reward_value_bp` 10000 =
  free), `free_shipping`, `flash_sale` (prices per variant with real units left), `tiered` (the highest
  tier reached), `bundle_price` ("3 items for 199,000"), `add_on` (a deal price for an extra item when
  a qualifying item is bought), `gift_choice` (the buyer picks gifts; free items come back as separate
  lines at price 0).
- Conditions: targets (products, variants, categories with their subcategories), customer groups,
  channels, first order only, minimum order or quantity, and extra conditions on payment method,
  carrier, province, membership tier, client (`web`, `app`, `pos`), seller, new / returning customers,
  items already discounted, excluded targets; time windows per weekday in the store's time zone.
- Targets, flash-sale prices and translations have their own endpoints
  (`PUT /promotions/{id}/targets`, `/flash-items`, `/translations/{locale}`).
- After a promotion is turned on, only a later end, more uses / budget / vouchers and its texts can
  change (`promotions.locked_after_start`); every change is kept in `GET /promotions/{id}/versions`.

## Turning on and the law

`POST /promotions/{id}/activation-check` lists the checks without changing anything;
`POST /promotions/{id}/activate` runs the same checks and fails with every failed check in `errors`:

- the discount limit in force (50% of the price just before the promotion in Viet Nam, unlimited for a
  national promotion programme), using the catalog's price history. Decree 239/2026/ND-CP (26 June 2026)
  repealed the limits of Decree 81/2018/ND-CP Articles 6 and 7 and left them to the Ministry of Industry and
  Trade; the circular is not found yet, so pack `vn` version 2 keeps 50% as a cautious default with the new
  basis, and keeps the 120 discount days a year that one summary says the decree dropped — check the original
  texts;
- discount days per programme and per year;
- the promotion notice recorded for each authority (`PATCH /promotions/{id}/notices/{authority}`; the
  province comes from the unit setting `promotions.notice_province`);
- targets, and the catalog's price floors (`block` stops, `warn` warns); conflicts are warnings.

Limits and notice rules are **data with effective dates**, not code: load them with
`autoxide task promotions.load_legal_data` (the pack `vn` ships with the server, with its legal basis and
"to be checked" notes) and read them at `GET /legal-limits?at=` and `GET /notice-rules?at=`. Without data
in force a promotion cannot be turned on (`promotions.no_legal_data`). The pack was compiled from
secondary sources: have it checked by a legal professional before real use.

To prepare the notices, `GET /promotions/{id}/notice-export?format=json|csv` (permission
`promotions.compliance` and `promotions.read`; MCP tool `promotions.export_notice`) gives one entry per authority: whether a notice
is required under the rules in force on the start day, the last day to submit, the legal basis and what was
recorded. People submit the notice to the authority; Autoxide only prepares the data and records it.

## Computing a cart

`POST /evaluate` (and `Promotions::evaluate`) computes a cart without holding anything:

1. candidates: running, in their window, of the store or its parents (a seller's promotions only on the
   seller's items), conditions met; coupons and vouchers open their promotions — a wrong code or voucher
   comes back in `rejected_codes` with a reason, the cart still works;
2. the `product` class, then `order` on what is left, then `shipping`. Two promotions stack only when
   both allow it (`exclusivity`, `combines_with`); `stop_after` stops lower priorities in the class. The
   unit setting `promotions.stacking_mode` picks `priority` (default, predictable) or `best_for_buyer`;
3. the legal limit per line adds every discount (all classes, platform and seller vouchers together);
   the later discount is cut first (`capped_by_law`), then the price floor (`capped_by_floor`). A product
   with no price before the promotion gets no discount (`withheld`);
4. each discount is shared among its payers (`funding`: platform, seller, brand, payment partner) to the
   cent; reasons are translated to the cart's `locale`.

Percentages are rounded half up on the line total; order discounts are shared to lines in proportion
to their amounts. The same cart always gives the same result and `fingerprint`.

A website menu can point to a promotion: the type `promotions.promotion` in the resource directory is public only
while a `public` promotion runs, with its title, conditions and the standard path `/promotions/<id>`.

## Sellers in platform promotions

On a marketplace, a seller joins a platform flash sale that has not started: `POST /promotions/{id}/participations`
(permission `promotions.participate` on the seller's unit, below the unit running the promotion) with the seller's
own products, each with a sale price or a discount and the units offered. The platform approves or rejects it
(`POST …/participations/{participation_id}/approve` or `/reject`, permission `promotions.publish`); approved products
join the flash sale and their discount is paid by the seller (`funding`). `GET …/participations` shows every
application to the platform and their own to sellers.

## Planning

All read-only, permission `promotions.read`:

- `POST /promotions/{id}/simulate` `{"carts": [...]}` (1 to 20 carts, MCP tool `promotions.simulate`): each cart
  computed with the promotion (a draft works too) and without it, and `difference_minor`;
- `GET /promotions/{id}/conflicts`: other promotions of the unit at the same time, on shared targets and in the
  same class, where one side is exclusive (only one of them applies to a line);
- `GET /promotions/calendar?from=&to=&org_id=`: promotions running on those days (Vietnam dates, up to 366);
- `GET /promotions/{id}/stats?from=&to=`: confirmed uses, discount, order value and who paid, per day.

## Holding usage

`Promotions::reserve` recomputes the cart and holds, in one transaction, every counter: uses of the
promotion and of the code, uses per buyer key (account, hashed phone, hashed address — a new account
with the same phone gets no extra use), uses per day, budget, flash-sale units, gifts and vouchers.
All or nothing; the same order and cart again returns the same hold. A different result from the one
the buyer saw fails with `promotions.changed`. `confirm` makes it final; `release` gives the usage back
(after confirmation only for promotions with `return_usage_on_cancel`; vouchers return to the wallet
while valid). Holds not confirmed expire (`promotions.expire_holds`, every minute).

## Vouchers

Buyers save the voucher of a `claimable` promotion (`POST /public/promotions/{id}/claim`), enter a claim
code (`POST /me/vouchers/claim-code`, tries are limited) or receive one through `grant_voucher` (loyalty,
birthday, compensation…; the same `source` + `source_ref` never gives twice) or by hand with a reason
(`POST /promotions/{id}/vouchers`). `GET /me/vouchers` lists the wallet. The wallet is in the personal
data export; deleting an account revokes unused vouchers.

## On the website

`GET /public/promotions?locale=&site=` (no sign-in) lists the promotions marked `public` running now on a
website: those of its unit and of the units above it, with their full conditions, the notices recorded with
the Departments of Industry and Trade (authority and reference) and the largest discount the law in force today
allows (`legal_limit`, from the law data). Pages with `limit` / `cursor`; send the `ETag` back in
`If-None-Match` to get `304`.

`GET /public/promotions/for-product/{product_id}?variant_id=&site=` lists the public coupons that lower the price of
one variant, cheapest first: the price of one unit with the coupon, its conditions and the uses left (only when
there is a real limit). The listed price stays the price without a coupon — show "X with code …", not a
discounted price. For a cart, `Promotions::eligible` (served by `orders`) gives the coupons and vouchers that work,
those that do not yet with a reason ("add X to use"), and the best selection, so a picker can replace the code box.

`GET /public/promotions/flash-sales?site=` lists the public flash sales running now or starting within a day, with
their products, sale prices and the units left at the sale price (a real count: held and sold units are taken off).
A signed-in buyer can ask to be reminded before one starts (`POST /public/promotions/{id}/reminders`, permission
`promotions.use`; `DELETE` cancels): one reminder is sent through `notifications` ten minutes before the start. Asking
is recorded in the consent ledger as consent to that one reminder, not to advertising.

## Returns

`api::returns` has the rules for `orders`: the refund of a line is the amount paid × returned / bought,
the last unit taking the remainder; order discounts are taken back only when the unit setting
`promotions.clawback_on_return` is `proportional`; gifts follow the promotion's `gift_return_policy`.

## Configuration

| Variable | Default | |
|---|---|---|
| `AUTOXIDE_PROMOTIONS_MAX_CODES_PER_CART` | 5 | codes per cart |
| `AUTOXIDE_PROMOTIONS_MAX_HOLD_HOURS` | 72 | longest hold |
| `AUTOXIDE_PROMOTIONS_CODE_LENGTH` | 10 | length of generated codes |
| `AUTOXIDE_PROMOTIONS_MAX_VOUCHERS_PER_CLASS` | 1 | vouchers per class and payer |
| `AUTOXIDE_PROMOTIONS_COMPILED_TTL_SECONDS` | 60 | how soon another server sees changes |
| `AUTOXIDE_PROMOTIONS_JURISDICTION` | `VN` | law data to apply |
