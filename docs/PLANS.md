# Plans, subscriptions and limits

[Tiếng Việt](PLANS.vi.md)

The `plans` module lets you sell Autoxide by plan without changing code. A **plan** is data: which
modules a tenant may use, which features are on, how much it may use (limits), and its prices.
Tenants get a plan through a **subscription**; the kernel enforces it on every request, MCP tool,
job and event.

Nothing changes until you create a plan: a tenant with no subscription gets the **default plan**,
and with no default plan it is not limited at all. The platform tenant (the operator) is never
limited.

Everything below is done by platform staff — users of the platform tenant holding
`plans.plans.manage` / `plans.subscriptions.manage` — over REST (`/api/v1/plans/...`) or MCP
(`plans.*` tools).

## Create a plan

See which modules, features and limits exist in `GET /api/v1/kernel/modules` (`features`,
`limits`, `always_enabled`, `requires`). Modules marked `always_enabled` (sign-in, files, audit…)
are on for everyone.

```bash
curl -X POST https://api.example.com/api/v1/plans/plans \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"code": "pro", "name": "Pro", "status": "active", "trial_days": 14,
       "modules": ["content", "media", "forms"],
       "features": ["content.ai_suggestions", "auth.api_keys"],
       "limits": {"organizations.units": 10, "auth.members": 25,
                  "files.storage_bytes": 10737418240, "kernel.api_requests": null}}'
```

- A limit set to `null` is unlimited; a limit left out is `0`.
- A base plan that turns on a module must turn on the modules it needs (`requires`), or the
  request answers `plans.module_dependency_missing`.
- `"is_default": true` makes it the plan of tenants without a subscription (only one plan is the
  default).
- `"kind": "addon"` makes an add-on: its limits are added on top of the base plan, times the
  quantity bought.
- Lifecycle settings per plan: `grace_days` (7), `suspend_days` (30), `retention_days` (90),
  `renewal_lead_days` (7).

Add a price with `POST /api/v1/plans/plans/{id}/prices`
(`{"kind": "recurring", "interval": "month", "amount": {"amount": "199000", "currency": "VND"}}`).
Prices never change: add a new one and deactivate the old one (`POST /prices/{id}/deactivate`);
subscriptions keep their price until they change plan. A plan is never deleted, only retired
(`POST /plans/{id}/retire`).

## Put a tenant on a plan

```bash
curl -X PUT https://api.example.com/api/v1/plans/tenants/$TENANT/subscription \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"plan_id": "...", "price_id": "...", "addons": [{"plan_id": "...", "quantity": 2}]}'
```

The first time this starts the plan's trial (unless `"trial": false`, or the tenant already had
one). For one tenant you can also turn a module or feature on or off, or replace a limit, with a
reason and an optional end: `POST /api/v1/plans/tenants/{id}/overrides`.

Changes take effect at once on the server that made them and within
`AUTOXIDE_PLANS_CACHE_SECONDS` (default 30) on the others.

## What tenants see

- `GET /api/v1/plans/entitlements` (any signed-in user): modules, features, each limit with its
  use, and `access` (`full` or `read_only`) — user interfaces hide what the plan does not include.
- An endpoint of a module the plan does not include answers `403 module_not_enabled`; a feature
  `403 feature_not_enabled`; a full limit `403 limit_exceeded`; the monthly request quota
  `429 quota_exhausted` (only when the plan sets a maximum).
- With `plans.billing.read` / `plans.billing.manage`: `GET /subscription`, `GET /usage`,
  `GET /billing-periods`, `POST /subscription/change` (an upgrade applies at once and bills the
  rest of the period; a downgrade applies when the period ends), `POST /subscription/cancel`,
  `POST /subscription/resume`.

## Lifecycle

| Status | Access | Next |
| --- | --- | --- |
| `trialing` | full | trial ends: free plan → `active`; priced → paid next period → `active`, else `past_due` |
| `active` | full | period ends: next period paid (or free) → new period; unpaid → `past_due` |
| `past_due` | full (grace) | paid → `active`; `grace_days` pass → `suspended` |
| `suspended` | read-only | paid → `active`; `suspend_days` pass → `canceled` |
| `canceled` | read-only | resumed before `delete_after` → `active`; `retention_days` pass → `deleted` |
| `deleted` | none | the tenant is suspended; its data is kept until you delete it |

A job moves subscriptions every 5 minutes. `renewal_lead_days` before a period ends, the next
billing period is opened and a reminder is mailed to the subscription's billing email
(`AUTOXIDE_PLANS_REMINDER_DAYS`, default `7,1`), again when overdue and every 3 days of the grace
period.

## Payments (manual for now)

Until a payment provider is connected (step 36), billing periods are paid outside the system:
check the bank transfer, then `POST /api/v1/plans/billing-periods/{id}/mark-paid`
(`{"provider_reference": "..."}`); a past-due or suspended subscription becomes active again.
`POST /billing-periods/{id}/void` cancels a period opened by mistake.

To block a tenant completely (reads too), use `POST /api/v1/plans/tenants/{id}/subscription/suspend`
with a reason; undo it with `PATCH /api/v1/kernel/tenants/{id}` (`"status": "active"`).
`GET /api/v1/kernel/tenants?subscription_status=past_due` and `?near_limit=true` list tenants to
follow up.

## Settings

| Setting | Default | |
| --- | --- | --- |
| `AUTOXIDE_PLANS_CACHE_SECONDS` | `30` | How long each server keeps a tenant's entitlements in memory |
| `AUTOXIDE_PLANS_HOLD_MINUTES` | `15` | How long an unconfirmed reservation against a limit is held |
| `AUTOXIDE_PLANS_REMINDER_DAYS` | `7,1` | Days before a period is due that a reminder is mailed |
| `AUTOXIDE_JOBS_BLOCKED_DELAY_SECONDS` | `3600` | How long a job of a module the plan excludes waits |
