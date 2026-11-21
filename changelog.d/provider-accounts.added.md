- Partner connections per tenant (provider adapters): each tenant's own webhook address at
  `GET /api/v1/kernel/connections/webhooks`, received at `POST /api/v1/ledger/webhooks/{provider}/{hook}` and
  `POST /api/v1/ledger/bank-feeds/{provider}/{hook}`; new MCP tools `kernel.set_setting` (not for secrets),
  `kernel.delete_setting`, `kernel.test_connection`, `kernel.list_connection_webhooks`; new error codes
  `provider.not_configured` and `provider.webhook_unknown`. Guide: `docs/guides/provider-adapters.md`.
