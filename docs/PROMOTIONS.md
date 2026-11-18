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
  national promotion programme), using the catalog's price history;
- discount days per programme and per year;
- the promotion notice recorded for each authority (`PATCH /promotions/{id}/notices/{authority}`; the
  province comes from the unit setting `promotions.notice_province`);
- targets, and the catalog's price floors (`block` stops, `warn` warns); conflicts are warnings.

Limits and notice rules are **data with effective dates**, not code: load them with
`autoxide task promotions.load_legal_data` (the pack `vn` ships with the server, with its legal basis and
"to be checked" notes) and read them at `GET /legal-limits?at=` and `GET /notice-rules?at=`. Without data
in force a promotion cannot be turned on (`promotions.no_legal_data`). The pack was compiled from
secondary sources: have it checked by a legal professional before real use.

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
