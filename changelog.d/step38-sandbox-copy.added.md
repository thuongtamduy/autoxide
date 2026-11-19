- A new sandbox starts with a copy of its tenant's configuration: settings that are not secret,
  roles, pages (published ones stay published) and the notification templates in use. Users,
  buyers, orders, documents, files and secrets are never copied.
- `POST /api/v1/kernel/sandboxes/{id}/promote` (MCP `kernel.promote_sandbox`) brings records tried
  in a sandbox back to the real tenant as drafts — never published, never overwriting what is live;
  `GET /api/v1/kernel/sandboxes/promotable` lists the kinds (`content.page` for now). Modules take
  part through the new `Module::sandbox_copy` and `Module::promotable` hooks.
