# Customers

The `customers` module keeps a sales profile for every buyer of a shop, builds the purchase history
from order events, and groups customers for price lists and promotions. It is registered after
`orders` and works without it (no history, no groups in carts).

## Profiles

- One profile per buyer in a shop, keyed by the buyer **account** and/or the **phone number**.
  The phone is stored next to a keyed hash (HMAC with a per-shop key, the same `buyer_key` that
  `orders` writes), so the same number matches inside one shop and never across shops.
- A profile is created on the buyer's first order (`orders.order_placed`), when staff register a
  customer at the counter (`POST /api/v1/customers`), or on the buyer's first
  `GET /api/v1/customers/me`.
- A profile kept by phone joins a buyer account **only when the account's phone number is
  verified** (one-time code, `compliance`). Registering a number at the counter does not give the
  history to whoever types that number later.
- `org_id` is the unit where the customer registered or first bought. Staff with `customers.read`
  at `unit` / `subtree` scope see only the customers of their units; a `tenant` scope sees all.

## Purchase history and totals

`customers` copies a summary of every order from the events of `orders` (it never reads the
orders tables). `GET /api/v1/customers/{id}/orders` lists them.

- **Total spent** = what the buyer paid for goods of **completed** orders, minus refunds: after all
  discounts (coupons and vouchers included), with tax, without shipping, without the part paid with
  points or gift cards. The definition is fixed (decision D-226).
- **Orders count** = orders placed and not cancelled.
- Events delivered twice never count twice.

## Masked contact details

Every list and detail for staff (REST, MCP tools, exports) shows `phone_masked` (`09•• ••• 412`)
and `email_masked` (`n•••@gmail.com`). To see them in full, call
`GET /api/v1/customers/{id}/contact?reason=…` with permission `customers.contact.reveal`; every call
emits `customers.contact_revealed`, which the audit log keeps (who, which customer, the reason).
The buyer sees their own profile unmasked at `GET /api/v1/customers/me`.

## Customer groups

- **Manual** groups: staff add and remove members (`POST` / `DELETE
  /api/v1/customers/groups/{id}/members/{profile_id}`).
- **Rule** groups: members follow a rule, all conditions must hold:

  ```json
  {"total_spent_min": 5000000, "orders_count_min": 3, "last_order_within_days": 180,
   "tier_in": ["gold"], "tags_any": ["<tag id>"], "org_in": ["<unit id>"]}
  ```

  Members are updated shortly after a profile changes and again every night (rules about time,
  like "ordered in the last 180 days", change without any event). `GET /groups/{id}/preview` counts
  the matching customers now.
- A group of a unit only matches customers of that unit and below.
- `price_group` (e.g. `vip`) is the customer group code that price lists of `catalog` use. When a
  buyer is in several groups with a code, the group first by name wins.
- Carts of `orders` receive the buyer's group ids (for promotions limited to groups), the
  membership tier and the price group.

## Tags, notes, custom fields

Profiles are records of kind `customers.customer`: internal tags, notes and custom fields come from
the shared kernel endpoints (`/api/v1/kernel/tags`, record notes, custom field definitions with
target `customers.customer`). Filter the list by tags with `?tags=<id>,<id>`.

## Permissions

| Permission | Allows |
|---|---|
| `customers.read` | See profiles in scope (masked), purchase history, groups |
| `customers.manage` | Register at the counter, edit profiles, tag profiles |
| `customers.contact.reveal` | See full phone and email (audited) |
| `customers.segments.manage` | Create, change, delete groups and their members |

Buyers need no permission: `/me` only needs a signed-in account.

## Personal data

The profile, purchase history and group memberships are part of the user's data export. When an
account is deleted, the profile loses every personal field (name, phone, email, birthday, custom
fields) and its link to the account; totals and order summaries stay without anything that
identifies the person. Legal basis to check with a qualified person: Personal Data Protection Law
91/2025/QH15 and Decree 356/2025.
