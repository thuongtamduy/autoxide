- AI agents (API keys of kind `agent`, MCP / OAuth clients) now wait for a person's approval (`202`, `x-approval:
  "agent"`) before every `DELETE` and every command that touches money or cannot be undone: refunds, payouts, issuing
  or cancelling e-invoices, sending newsletter campaigns, erasing or moving tenants, voiding gift cards, closing ledger
  periods, and more. The matching MCP tools answer `approval_required`. A command that needs no permission is approved
  by the agent's owner. `docs/mcp-tools.json` now shows `requires_approval_for_agent` truthfully. New gate rules
  `api-approval` and `mcp-approval` keep new endpoints and tools covered.
