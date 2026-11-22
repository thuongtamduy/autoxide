# Guides

Task-by-task guides to the Autoxide API. Each has `curl` examples, which run as tests against every build,
and TypeScript examples with the SDK in [`sdk/typescript`](../../sdk/typescript/).

Vietnamese: [`README.vi.md`](README.vi.md).

The `curl` examples send JSON with `--json '…'`, which needs curl 7.82 or newer (`curl --version`). With an
older curl, replace `--json '…'` with `-H 'Content-Type: application/json' -H 'Accept: application/json' -d '…'`.

- [Showing posts and pictures on a website](website.md): public endpoints, `srcset` and WebP, ETag and 304
- [Building a shop's home page from the API](storefront.md): placements and banners, collections, brand and contact details, cache
- [Signing in and staying signed in](authentication.md): access and refresh tokens, signing out
- [Signing in with Google, Microsoft, GitHub or OpenID Connect](oidc.md): providers per tenant, linking accounts safely
- [Syncing staff from your identity provider (SCIM 2.0)](scim.md): Entra ID, Okta and Google Workspace create, change and disable staff accounts
- [API keys for programs and AI agents](api-keys.md): least permissions, rate limit, revoking
- [Retrying safely](retries.md): `Idempotency-Key`
- [Reading long lists](pagination.md): `limit`, `cursor`, `order`
- [Reading errors](errors.md): `code`, `errors`, `Retry-After`
- [Connecting an AI agent over MCP](mcp.md): a key for the agent, client setup, the tools
- [Receiving events in real time](realtime-events.md): Server-Sent Events, `Last-Event-ID`, limits
- [Uploading large files straight to storage](direct-uploads.md): signed links to S3, completing, checks
- [Trying changes in a sandbox](sandbox.md): a test copy of your tenant, simulated partners, bringing drafts back
- [Automating work with rules](automations.md): triggers, conditions, MCP tools as actions, approvals, alerts on unusual figures
- [Building a mobile or desktop app](mobile.md): SDKs, tokens and PKCE, push, offline sync, WebSocket, in-app purchases
- [Writing a provider adapter](provider-adapters.md): per-tenant accounts, secrets, webhooks, testing without the network (for developers)
- [Confirming bank transfers with SePay](sepay.md): your own SePay account per tenant, the webhook address, VietQR payments confirmed by themselves (experimental)
- [Issuing e-invoices through SePay eInvoice](sepay-einvoice.md): your own SePay eInvoice account per tenant, what is sent to the tax authority and what is not (experimental)
