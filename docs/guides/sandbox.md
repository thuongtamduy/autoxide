# Trying changes in a sandbox

A sandbox is a test copy of your tenant. Change pages, roles, templates or settings there, try
payments and notifications without touching real customers or real partners, then bring what you
like back as **drafts**. Sandboxes exist when the server runs several tenants
(`AUTOXIDE_TENANCY_MODE=multi`).

Vietnamese: [`sandbox.vi.md`](sandbox.vi.md).

## Create a sandbox

You need `kernel.sandboxes.manage` in your tenant. Your plan sets how many sandboxes you may keep at
the same time (`kernel.sandboxes`, 1 when the plan does not say).

```bash
curl -sS "$API/api/v1/kernel/tenants/$TENANT/sandboxes" \
  -H "Authorization: Bearer $TOKEN" -H "Idempotency-Key: $(uuidgen)" \
  --json '{"name": "Try the new theme", "role_id": null}'
```

The answer is `201` with the sandbox (`status: "provisioning"`) and `job_id`. Follow the job at
`GET /api/v1/kernel/jobs/{job_id}`; when it is done the sandbox is `active`.

**What is copied:** settings that are not secret, roles, pages (published pages stay published), the
notification templates in use. **What is never copied:** users, buyers, orders, ledger, documents,
notifications, private files, secrets and partner connection details, domains, API keys.

## Work inside it

Holders of `role_id` in your tenant (the `admin` role by default) enter the sandbox with one header;
there is no separate account:

```bash
curl -sS "$API/api/v1/kernel/tenant" \
  -H "Authorization: Bearer $TOKEN" -H "Autoxide-Tenant: $SANDBOX"
```

Every answer from a sandbox carries `Autoxide-Sandbox: true` and `X-Robots-Tag: noindex, nofollow`.
Its public website has an empty sitemap and shows a "test copy" label instead of the ministry badge.

Inside a sandbox:

- **Partners are always simulated.** Payments succeed, payouts succeed, e-invoices get numbers
  starting with `SANDBOX-`, identity checks pass. Nothing reaches a bank, a carrier, a tax provider
  or an SMS gateway, whatever your tenant has set up. These results are never legal evidence.
- **Mail** reaches only the verified e-mail of staff who may enter the sandbox. Mail to anyone else is
  kept, not sent.
- **Webhooks** go only to URLs that start with a prefix in the tenant setting
  `kernel.sandbox.webhook_urls` (set it inside the sandbox; empty by default). Other deliveries get
  the status `sandboxed`.

Everything kept back is listed in the sandbox message box:

```bash
curl -sS "$API/api/v1/kernel/sandbox/messages?channel=mail" \
  -H "Authorization: Bearer $TOKEN" -H "Autoxide-Tenant: $SANDBOX"
```

### Complete a simulated payment

No partner notifies a sandbox, so a payment made there (an order paid by `card` or `wallet`, for
example) stays `pending` until you play the partner. The server operator first allows it with
`AUTOXIDE_LEDGER_SANDBOX_SIMULATION=true` (off by default; it never works outside a sandbox). Then,
with `ledger.reconcile`:

```bash
# The order's payment action gives the id as `ledger_payment_id`.
curl -sS "$API/api/v1/ledger/payments/$LEDGER_PAYMENT_ID/simulate" \
  -H "Authorization: Bearer $TOKEN" -H "Autoxide-Tenant: $SANDBOX" \
  -H "Idempotency-Key: $(openssl rand -hex 16)" -H 'Content-Type: application/json' \
  -d '{"outcome": "succeeded"}'
```

It goes through the same path as a partner notification: the payment becomes `succeeded` and the
order `confirmed` a moment later. `{"outcome": "failed"}` makes the payment fail; an `amount` other
than the amount due (`{"outcome": "succeeded", "amount": {"amount": "100000", "currency": "VND"}}`)
puts it in `needs_review`, to practise resolving a difference. Only `pending` payments created inside
the sandbox can be simulated. AI agents use the MCP tool `ledger.simulate_payment`.

## Bring changes back

From your real tenant (without `Autoxide-Tenant`), promote records of the sandbox. List the kinds
with `GET /api/v1/kernel/sandboxes/promotable`.

```bash
curl -sS "$API/api/v1/kernel/sandboxes/$SANDBOX/promote" \
  -H "Authorization: Bearer $TOKEN" -H "Idempotency-Key: $(uuidgen)" \
  --json '{"items": [{"kind": "content.page", "id": "'"$PAGE"'"}]}'
```

Each record gets an outcome:

| Outcome | Meaning |
| --- | --- |
| `created_draft` | A new draft in your tenant (the record was made in the sandbox) |
| `updated_draft` | The draft of the original was replaced; what is live is unchanged until you publish |
| `unchanged` | Already promoted with the same content |
| `conflict` | A slug, name or code is taken in your tenant; see `conflict.field` |

Nothing is published or enabled for you, and you need the permission of the module too (for example
`content.write` for a page). Agents do the same with the MCP tools `kernel.list_promotable` and
`kernel.promote_sandbox`.

## Extend or delete

`POST /api/v1/kernel/sandboxes/{id}/extend` gives it another `AUTOXIDE_TENANCY_SANDBOX_DAYS` days
(30 by default) from now. `DELETE /api/v1/kernel/sandboxes/{id}` deletes it and everything tried in
it at once; promote first what you want to keep.
