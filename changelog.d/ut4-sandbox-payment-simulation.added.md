- Module `ledger`: `POST /api/v1/ledger/payments/{id}/simulate` (MCP tool `ledger.simulate_payment`)
  completes a simulated payment inside a sandbox, through the same path as a partner notification
  (`succeeded`, `failed`, or `needs_review` for another amount). Off unless the operator sets
  `AUTOXIDE_LEDGER_SANDBOX_SIMULATION=true`; only for `pending` payments created inside the sandbox.
- Testkit: `McpClient::with_header` sends an extra header (e.g. `Autoxide-Tenant`) with every MCP request.
