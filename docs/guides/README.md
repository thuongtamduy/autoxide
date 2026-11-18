# Guides

Task-by-task guides to the Autoxide API. Each has `curl` examples, which run as tests against every build,
and TypeScript examples with the SDK in [`sdk/typescript`](../../sdk/typescript/).

Vietnamese: [`README.vi.md`](README.vi.md).

- [Showing posts and pictures on a website](website.md): public endpoints, `srcset` and WebP, ETag and 304
- [Signing in and staying signed in](authentication.md): access and refresh tokens, signing out
- [Signing in with Google, Microsoft, GitHub or OpenID Connect](oidc.md): providers per tenant, linking accounts safely
- [API keys for programs and AI agents](api-keys.md): least permissions, rate limit, revoking
- [Retrying safely](retries.md): `Idempotency-Key`
- [Reading long lists](pagination.md): `limit`, `cursor`, `order`
- [Reading errors](errors.md): `code`, `errors`, `Retry-After`
- [Connecting an AI agent over MCP](mcp.md): a key for the agent, client setup, the tools
- [Receiving events in real time](realtime-events.md): Server-Sent Events, `Last-Event-ID`, limits
- [Uploading large files straight to storage](direct-uploads.md): signed links to S3, completing, checks
- [Automating work with rules](automations.md): triggers, conditions, MCP tools as actions, approvals, alerts on unusual figures
