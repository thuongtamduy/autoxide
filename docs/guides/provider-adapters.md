# Writing a provider adapter

An adapter connects Autoxide to one outside service: a payment gateway, a bank feed, a carrier, an SMS
provider, an e-invoice provider. This guide is for developers who add one. Every adapter is built the
same way, so that a tenant sets up any of them with the same API.

Vietnamese: [`provider-adapters.vi.md`](provider-adapters.vi.md).

## The rules

1. **One small module crate per adapter**, `modules/<name>`, that depends only on the kernel (and on
   the module that publishes the contract, when the contract is not in the kernel, e.g.
   `autoxide_shipping::api`). It implements a contract (`PaymentGateway`, `BankFeed`, `SmsSender`,
   `EInvoiceProvider`, `ShippingCarrier`…) and is registered before the module that uses it.
2. **The account at the provider belongs to the tenant.** API keys, shop ids and webhook secrets are
   tenant settings in the `connection` group, never environment variables. Two tenants on one server
   use two different accounts. A tenant that has not set its account gets the error
   `provider.not_configured`; an adapter never falls back to another account.
3. **Secrets are write-only.** A secret setting is stored encrypted, is never returned by the API, and
   never appears in logs, events or errors. People set secrets over REST; the MCP tool
   `kernel.set_setting` refuses secret settings, so a secret never passes through an agent.
4. **Written from the provider's public documentation.** Do not write from memory. Put the URL of each
   page you used, and the date you read it, in the comment at the top of the module. Until the adapter
   has run against the real service, the crate description and its documentation say so: *"written
   from the provider's public documentation; not yet verified against the live service"*. Adapters are
   experimental until then.
5. **No network call inside a write transaction**, and every outbound call has a timeout.
6. **Works on SQLite and PostgreSQL**, like everything else. Most adapters have no table at all.

## 1. Declare the account

```rust
use autoxide_kernel::tenancy::{CONNECTION_GROUP, TenantSetting};

// One setting per field of the tenant's account: `<module>.connection.<kind>.<field>`.
pub(crate) const API_KEY: TenantSetting = TenantSetting::new(
    "acme.connection.api.key",
    "API key of this tenant's Acme account (Acme dashboard > Developers).",
)
.secret()
.group(CONNECTION_GROUP);

pub(crate) const SHOP_ID: TenantSetting = TenantSetting::new(
    "acme.connection.api.shop_id",
    "Shop id of this tenant's Acme account.",
)
.group(CONNECTION_GROUP);
```

Return them from `Module::tenant_settings`. The tenant then manages them with the API that already
exists:

| What | REST | MCP tool |
| --- | --- | --- |
| See what a connection needs and what is set | `GET /api/v1/kernel/settings?group=connection` | `kernel.list_settings` |
| Set a field | `PUT /api/v1/kernel/settings/{name}` | `kernel.set_setting` (not for secrets) |
| Remove a field | `DELETE /api/v1/kernel/settings/{name}` | `kernel.delete_setting` |
| Test the connection | `POST /api/v1/kernel/settings/{name}/test` | `kernel.test_connection` |
| See the webhook addresses | `GET /api/v1/kernel/connections/webhooks` | `kernel.list_connection_webhooks` |

All of them need the permission `kernel.settings.manage`, which is marked as sensitive. Setting or
removing a `connection` setting also needs a recent sign-in (`step_up_required` otherwise), because it
changes where money or data goes; every change is in the audit log, by name only.

A setting that holds an address chosen by the tenant must be checked when it is stored
(`.checked(…)` with `autoxide_kernel::net::check_target_url`) and the client must use
`Target::TenantChosen`, so the server only connects to public addresses.

## 2. Read the account when called

```rust
use autoxide_kernel::provider::ProviderAccounts;

// In `Module::start`:
let accounts = context.require::<ProviderAccounts>()?;

// In every trait method (the traits pass `TenantId`):
let tenant = self.accounts.tenant(tenant_id).await?;
let key = self.accounts.require(&tenant, &API_KEY).await?; // `provider.not_configured` when unset
let shop = self.accounts.get(&tenant, &SHOP_ID).await?; // optional field
```

## 3. Call the provider

```rust
use autoxide_kernel::provider::{Target, http_client, send};

let client = http_client(Duration::from_secs(15), Target::Fixed)?;
let request = client
    .post(format!("{}/v1/orders", self.base_url))
    .bearer_auth(key.as_str())
    .header("content-type", "application/json")
    .body(serde_json::to_vec(&body).map_err(Error::internal)?);
let reply = send(request, "acme", "create_order")
    .await
    .map_err(|_| payment::provider_unavailable())?;
if !reply.is_success() {
    return Err(payment::provider_rejected("…the partner's documented error code…"));
}
```

`send` logs the provider, the purpose and the HTTP status, never the address, headers or body, and
counts calls in `kernel_provider_calls_total`. The provider's own address is not a tenant setting:
declare it as an operator setting (`AUTOXIDE_<NAME>_API_URL`, with the provider's production address
as the default) so tests and sandboxes can point it elsewhere.

Send the id Autoxide gives you (`payment_id`, `document_id`, `reference`…) as the provider's own
idempotency or merchant reference, so a retried call does not create a second payment, parcel,
message or invoice.

Register a `ConnectionProbe` (`context.connection_probe`) that makes one cheap authenticated call, so
the tenant can test the account from the settings page. Report each use with
`context.connection_status().record(…)`; the message must not contain a secret.

## 4. Receive webhooks

Every tenant has its own webhook address. Its last part identifies the tenant and cannot be guessed;
the receiving module finds the tenant from it first, and the adapter then checks the notification with
**that tenant's** secret.

```rust
use autoxide_kernel::provider::{WebhookAck, WebhookEndpoint, constant_time_eq, hex, hmac_sha256};

// In `Module::start`: the tenant then finds its address in `GET /api/v1/kernel/connections/webhooks`.
accounts.register_webhook(WebhookEndpoint::new(
    "acme.connection.api",
    "/api/v1/ledger/webhooks/acme",
    "Paste into Acme dashboard > Webhooks > Endpoint URL.",
));

// In the trait:
async fn verify_webhook(&self, tenant: TenantId, headers: &HeaderMap, body: &[u8])
    -> Result<GatewayNotification, Error> {
    let tenant = self.accounts.tenant(tenant).await?;
    let secret = self.accounts.require(&tenant, &WEBHOOK_SECRET).await?;
    let wanted = hex(&hmac_sha256(secret.as_str().as_bytes(), body)); // on the RAW body
    let given = headers.get("x-acme-signature").and_then(|v| v.to_str().ok()).unwrap_or("");
    if !constant_time_eq(wanted.as_bytes(), given.as_bytes()) {
        return Err(payment::signature_invalid());
    }
    // …only now read the body…
}

fn webhook_ack(&self) -> Option<WebhookAck> {
    Some(WebhookAck::json(200, r#"{"received":true}"#)) // what the partner's documentation asks for
}
```

- Check the signature (or API key) in constant time, on the raw body, before reading anything.
- Give every notification a stable id (`event_id`, `provider_txn_id`): the receiving module stores each
  id once per tenant, so a notification sent again changes nothing.
- The address alone proves nothing. A provider that offers no signature or key at all cannot be
  connected safely; do not accept unauthenticated notifications.

The receiving routes are `POST /api/v1/ledger/webhooks/{provider}/{hook}` (payment gateways) and
`POST /api/v1/ledger/bank-feeds/{provider}/{hook}` (bank feeds). A module that receives another kind
of webhook adds a route of the same shape and resolves the tenant with
`ProviderAccounts::resolve_webhook`.

### The webhook address in a request

Some providers want the address in every request (an `ipnUrl`, a `CallbackUrl`). Ask for the tenant's
full address with `ProviderAccounts::webhook_url(&tenant, path)`: it is built from the server's
`AUTOXIDE_HTTP_PUBLIC_URL` and fails with `public_url_missing` when that is not set. Never build it from
the `Host` header of a request. `ProviderAccounts::webhook_token(&tenant, path)` gives only the last
part.

### Sandbox accounts

A provider's test account usually works only against the provider's sandbox address. When the provider
has one, declare a tenant setting `<module>.connection.<kind>.environment` checked with
`Environment::check` (`production`, the default, or `sandbox`), and two operator settings,
`AUTOXIDE_<NAME>_API_URL` and `AUTOXIDE_<NAME>_SANDBOX_API_URL`. The tenant chooses between the two
fixed addresses with `Environment::pick`; it never types an address. A kernel sandbox tenant never
reaches a real adapter at all: the kernel gives it the simulated partners.

## Carriers use the carrier accounts of `shipping`

A shipping adapter is different: the module `shipping` already keeps carrier accounts per tenant
(several per tenant, credentials encrypted), and `ShippingCarrier` receives the account in every call.
Such an adapter declares no `connection` settings and no `ConnectionProbe` — the account is tested with
`ShippingCarrier::check` — and uses only `http_client`, `send`, the signature helpers and the webhook
functions above.

## 5. Test without the network

```rust
use autoxide_testkit::{FakeProviderServer, TestApp, TestDatabase};

let server = FakeProviderServer::start().await.unwrap();
server.reply("POST", "/v1/orders", 200, json!({"id": "A1"}));
let app = TestApp::builder(test)
    .setting("AUTOXIDE_ACME_API_URL", server.url())
    .tenant("one")
    .tenant("two")
    // …
    .module(crate::module())
    .build()
    .await
    .unwrap();
// …each tenant sets its own account, call the adapter, then:
let seen = server.take_requests();
assert_eq!(seen[0].header("authorization"), Some("Bearer key-of-one"));
```

Test, on both databases: the request the provider's documentation describes (path, headers,
signature, body), the documented error answers, a webhook with a wrong signature, the same webhook
twice, and two tenants with two accounts that never mix.

## 6. Register and document

- Add the module to `crates/app/src/modules.rs`, before the module that uses the contract, and its
  crate to `Cargo.toml` and `crates/app/Cargo.toml`.
- Implement `Module::description`, `Module::stability` (`Stability::Experimental` until the adapter
  has run against the real service) and `Module::depends_on` (what `start` requires).
- Add the module's name to the experimental list in `crates/app/src/e2e/module_selection.rs`, a line
  to the table in `docs/MODULES.md` and `docs/MODULES.vi.md`, and its name to the module list expected
  in `crates/app/tests/integration/cli.rs`.
- Tests that need another module (`ledger`, `invoices`, `shipping`…) go in `crates/app/src/e2e/`; a
  module's own tests may not call another module's `module()`.
- Say in the adapter's guide how to turn it on: `AUTOXIDE_MODULES=official,<name>`.
- Add `locales/vi.json` for every error code of the adapter.
- Add a changelog fragment and, for a compliance feature, the legal basis in a decision record.
