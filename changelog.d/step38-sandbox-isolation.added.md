- Sandboxes are isolated: every partner contract (payments, payouts, bank feeds, SMS, Zalo, push,
  e-invoices, identity checks, mail) is wrapped by the kernel, so a sandbox always gets simulated
  results (`SANDBOX-…` references) and never reaches a real partner. Mail only reaches staff who
  may enter the sandbox, webhooks only URLs listed in `kernel.sandbox.webhook_urls` (otherwise the
  delivery is `sandboxed`), and everything held back is listed at
  `GET /api/v1/kernel/sandbox/messages` and the MCP tool `kernel.list_sandbox_messages`.
- A sandbox uses the plan of its real tenant, shows a "test copy" label instead of the ministry
  badge, has an empty public sitemap, and shipping in it always uses the simulated carrier.
