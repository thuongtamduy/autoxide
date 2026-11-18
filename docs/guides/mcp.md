# Connecting an AI agent over MCP

Vietnamese: [`mcp.vi.md`](mcp.vi.md). Examples use `curl` with `API` set to your server (`export API=https://api.example.com`), and the TypeScript SDK (`sdk/typescript`). They run as tests against every build, so they match the server of this version.

AI agents call Autoxide as tools through the [Model Context Protocol](https://modelcontextprotocol.io) at `/mcp` (Streamable HTTP). Each tool runs the same code as the matching REST endpoint, with the same permission: a tool can never do what REST with the same permission cannot.

## 1. Create a key with the least permissions

Give the agent its own API key, with only what its job needs. `content.write_own` lets it write drafts of its own; publishing stays with people.

<!-- example: sign-in -->
```bash
curl -sS "$API/api/v1/auth/login" \
  --json '{"email":"admin@example.com","password":"correct horse battery staple"}'
# save: TOKEN=.data.access_token
# save: REFRESH=.data.refresh_token
```

<!-- example: create-agent-key -->
```bash
curl -sS "$API/api/v1/auth/api-keys" -H "Authorization: Bearer $TOKEN" \
  --json '{"name":"Writing agent","permissions":["content.read","content.write_own"],"current_password":"correct horse battery staple"}'
# expect: 201
# save: AGENT_KEY=.data.key
```

<!-- example: agent-sees-its-permissions -->
```bash
curl -sS "$API/api/v1/auth/me" -H "Authorization: Bearer $AGENT_KEY"
```

## 2. Connect the client

Claude Code:

```bash
claude mcp add --transport http autoxide https://api.example.com/mcp --header "Authorization: Bearer $AUTOXIDE_AGENT_KEY"
```

Other MCP clients take a Streamable HTTP server with a header, e.g.:

```json
{
  "mcpServers": {
    "autoxide": {
      "type": "http",
      "url": "https://api.example.com/mcp",
      "headers": { "Authorization": "Bearer ${AUTOXIDE_AGENT_KEY}" }
    }
  }
}
```

## 3. What the agent can do

`tools/list` shows only the tools the key's permissions allow. Each description ends with the permission it needs, and annotations say whether a tool is read-only or destructive (clients ask before calling those). A deprecated tool says so in its description and in every result (`_meta.autoxide/deprecation`), with the tool to use instead.

Limits:

- Requests with an API key share its rate limit (`AUTOXIDE_HTTP_RATE_LIMIT_API_KEY_PER_MINUTE`).
- Every write lands in the audit log under the key.
- AI results are suggestions and drafts that a person reviews; nothing an agent writes is published by itself.
- Tool results have the same shape as REST: `{"data": …}`, or a page with `pagination`; errors carry the same `code`.

The tools of this version, by module (`docs/mcp-tools.json` has their full input schemas):

<!-- tools:start -->

| Tool | Permission | Kind |
|---|---|---|
| `address.list_communes` | — | read-only |
| `address.list_provinces` | — | read-only |
| `address.search_units` | — | read-only |
| `address.validate_address` | — | read-only |
| `ai.status` | `ai.view` | read-only |
| `audit.get_entry` | `audit.read` | read-only |
| `audit.list_entries` | `audit.read` | read-only |
| `auth.count_memberships` | `auth.users.read` | read-only |
| `auth.count_role_members` | `auth.roles.read` | read-only |
| `auth.create_api_key` | — | writes |
| `auth.create_invitation` | `auth.invitations.manage` | writes |
| `auth.create_role` | `auth.roles.manage` | writes |
| `auth.delete_role` | `auth.roles.manage` | destructive |
| `auth.get_role` | `auth.roles.read` | read-only |
| `auth.get_user_roles` | `auth.users.read` | read-only |
| `auth.list_api_keys` | — | read-only |
| `auth.list_invitations` | `auth.invitations.manage` | read-only |
| `auth.list_memberships` | `auth.users.read` | read-only |
| `auth.list_permissions` | `auth.roles.read` | read-only |
| `auth.list_roles` | `auth.roles.read` | read-only |
| `auth.list_users` | `auth.users.read` | read-only |
| `auth.replace_role` | `auth.roles.manage` | writes |
| `auth.revoke_api_key` | — | destructive |
| `auth.revoke_invitation` | `auth.invitations.manage` | writes |
| `auth.set_memberships` | `auth.roles.manage` | writes |
| `auth.set_user_roles` | `auth.roles.manage` | writes |
| `auth.update_user` | `auth.users.manage` | writes |
| `auth.whoami` | — | read-only |
| `comments.answer` | `comments.post` | writes |
| `comments.get` | `comments.read` | read-only |
| `comments.moderate` | `comments.moderate` | writes |
| `comments.queue` | `comments.read` | read-only |
| `comments.set_target_state` | `comments.manage` | writes |
| `comments.unanswered` | `comments.read` | read-only |
| `compliance.account_verification` | — | read-only |
| `compliance.add_case_note` | `compliance.cases.handle` | writes |
| `compliance.add_incident_entry` | `compliance.incidents.manage` | writes |
| `compliance.documents` | `compliance.documents.read` | read-only |
| `compliance.get_case` | `compliance.cases.read` | read-only |
| `compliance.incident_draft` | `compliance.incidents.manage` | writes |
| `compliance.incidents` | `compliance.incidents.manage` | writes |
| `compliance.kyc_profiles` | `compliance.kyc.read` | read-only |
| `compliance.legal_profiles` | `compliance.legal.read` | read-only |
| `compliance.list_cases` | `compliance.cases.read` | read-only |
| `compliance.open_incident` | `compliance.incidents.manage` | writes |
| `compliance.pending_acceptances` | — | read-only |
| `compliance.processing_register` | `compliance.privacy.read` | read-only |
| `compliance.publish_document_version` | `compliance.documents.manage` | writes |
| `compliance.purposes` | `compliance.privacy.read` | read-only |
| `compliance.record_subject_request` | `compliance.privacy.manage` | writes |
| `compliance.restrict_case_target` | `compliance.cases.handle` | writes |
| `compliance.retention` | `compliance.privacy.read` | read-only |
| `compliance.set_website_status` | `compliance.legal.manage` | writes |
| `compliance.subject_requests` | `compliance.privacy.manage` | read-only |
| `compliance.tracking_tags` | `compliance.tracking.manage` | writes |
| `compliance.verification_status` | — | read-only |
| `compliance.website_checklist` | `compliance.legal.read` | read-only |
| `compliance.website_dossier` | `compliance.legal.read` | read-only |
| `compliance.websites` | `compliance.legal.read` | read-only |
| `content.create_draft` | `content.write` or `content.write_own` | writes |
| `content.draft_from_template` | `content.write` or `content.write_own` | writes |
| `content.get` | `content.read` | read-only |
| `content.get_translation_request` | `content.read` or `content.write_own` | read-only |
| `content.legal_templates` | `content.read` | read-only |
| `content.list` | `content.read` | read-only |
| `content.list_revisions` | `content.read` | read-only |
| `content.publish` | `content.publish` or `content.publish_own` | writes |
| `content.request_review` | `content.write` or `content.write_own` | writes |
| `content.resolve` | `content.read` | read-only |
| `content.restore_revision` | `content.write` or `content.write_own` | writes |
| `content.search` | `content.read` | read-only |
| `content.seo_check` | `content.read` | read-only |
| `content.suggest_seo` | `content.write` or `content.write_own` | writes |
| `content.translate` | `content.write` or `content.write_own` | writes |
| `content.unpublish` | `content.publish` or `content.publish_own` | destructive |
| `content.update` | `content.write` or `content.write_own` | writes |
| `files.confirm_upload` | `files.upload` or `files.manage` | writes |
| `files.create_upload` | `files.upload` | writes |
| `files.delete_file` | `files.upload` or `files.manage` | destructive |
| `files.get_file` | `files.upload` or `files.manage` | read-only |
| `files.list_files` | `files.upload` | read-only |
| `files.read_file` | `files.upload` or `files.manage` | read-only |
| `files.upload_file` | `files.upload` | writes |
| `forms.create` | `forms.manage` | writes |
| `forms.get` | `forms.read` | read-only |
| `forms.get_submission` | `forms.read` | read-only |
| `forms.list` | `forms.read` | read-only |
| `forms.list_submissions` | `forms.read` | read-only |
| `forms.set_submission_status` | `forms.review` | writes |
| `forms.update` | `forms.manage` | writes |
| `invoices.create_draft` | `invoices.write` | writes |
| `invoices.create_export` | `invoices.export` | writes |
| `invoices.document` | `invoices.read` | read-only |
| `invoices.documents` | `invoices.read` | read-only |
| `invoices.export` | `invoices.export` | read-only |
| `invoices.issue` | `invoices.issue` | writes |
| `invoices.print` | `invoices.read` | read-only |
| `kernel.add_domain` | `kernel.domains.manage` | writes |
| `kernel.current_tenant` | — | read-only |
| `kernel.decide_approval` | — | writes |
| `kernel.export_tenant` | `kernel.data.export` | writes |
| `kernel.get_migration_run` | `kernel.tenants.operate` | read-only |
| `kernel.get_module` | `kernel.modules.read` | read-only |
| `kernel.get_tenant` | `kernel.tenants.read` | read-only |
| `kernel.get_tenant_export` | `kernel.data.export` | read-only |
| `kernel.get_tenant_move` | `kernel.tenants.read` | read-only |
| `kernel.list_approvals` | — | read-only |
| `kernel.list_custom_fields` | — | read-only |
| `kernel.list_domains` | `kernel.domains.manage` | read-only |
| `kernel.list_modules` | `kernel.modules.read` | read-only |
| `kernel.list_settings` | `kernel.settings.manage` | read-only |
| `kernel.list_tenants` | `kernel.tenants.read` | read-only |
| `kernel.migrate_placements` | `kernel.tenants.operate` | writes |
| `kernel.move_tenant` | `kernel.tenants.move` | destructive |
| `kernel.navigation` | — | read-only |
| `kernel.request_log` | `kernel.devtools.read` | read-only |
| `kernel.update_domain` | `kernel.domains.manage` | writes |
| `kernel.verify_domain` | `kernel.domains.manage` | writes |
| `ledger.balance` | `ledger.read` | read-only |
| `ledger.create_payout` | `ledger.payouts_create` | writes |
| `ledger.create_vietqr_payment` | `ledger.payments_create` | writes |
| `ledger.entries` | `ledger.read` | read-only |
| `ledger.exceptions` | `ledger.read` | read-only |
| `ledger.exchange_rate` | `ledger.read` | read-only |
| `ledger.flow_settings` | `ledger.read` | read-only |
| `ledger.holds` | `ledger.read` | read-only |
| `ledger.payment` | `ledger.read` | read-only |
| `ledger.payouts` | `ledger.read` | read-only |
| `ledger.post_entry` | `ledger.post` | writes |
| `ledger.record_exchange_rate` | `ledger.settings_manage` | writes |
| `ledger.release_hold` | `ledger.holds_release` | writes |
| `ledger.resolve_exception` | `ledger.reconcile` | writes |
| `ledger.trial_balance` | `ledger.read` | read-only |
| `media.confirm_upload` | `media.write` | writes |
| `media.create_upload` | `media.write` | writes |
| `media.get` | `media.read` | read-only |
| `media.list` | `media.read` | read-only |
| `media.list_tags` | `media.read` | read-only |
| `media.set_alt` | `media.write` | writes |
| `media.suggest_alt` | `media.write` | writes |
| `media.trash` | `media.write` | writes |
| `media.update` | `media.write` | writes |
| `media.upload` | `media.write` | writes |
| `menus.create_menu` | `menus.manage` | writes |
| `menus.delete_menu` | `menus.manage` | destructive |
| `menus.get_menu` | `menus.read` | read-only |
| `menus.list_menus` | `menus.read` | read-only |
| `menus.replace_menu_items` | `menus.manage` | writes |
| `newsletter.create_campaign` | `newsletter.manage` | writes |
| `newsletter.find_subscribers` | `newsletter.read` | read-only |
| `newsletter.get_campaign` | `newsletter.read` | read-only |
| `newsletter.list_lists` | `newsletter.read` | read-only |
| `newsletter.schedule_campaign` | `newsletter.send` | writes |
| `newsletter.send_campaign` | `newsletter.send` | writes |
| `notifications.get_template_schema` | — | read-only |
| `notifications.list_kinds` | — | read-only |
| `notifications.list_messages` | `notifications.messages.read` | read-only |
| `notifications.list_my_devices` | — | read-only |
| `notifications.list_my_inbox` | — | read-only |
| `notifications.mark_inbox_read` | — | writes |
| `notifications.preview_template` | `notifications.templates.read` or `notifications.templates.manage` | read-only |
| `notifications.send_notification` | `notifications.send` | writes |
| `notifications.update_template_draft` | `notifications.templates.manage` | writes |
| `organizations.change_unit_status` | `organizations.units.manage` | destructive |
| `organizations.create_unit` | `organizations.units.write` | writes |
| `organizations.get_settings` | `organizations.units.read` | read-only |
| `organizations.get_unit` | `organizations.units.read` | read-only |
| `organizations.list_units` | `organizations.units.read` | read-only |
| `organizations.move_unit` | `organizations.units.write` | writes |
| `organizations.set_setting` | `organizations.settings.write` | writes |
| `organizations.update_unit` | `organizations.units.write` | writes |
| `plans.assign_subscription` | `plans.subscriptions.manage` | writes |
| `plans.change_subscription` | `plans.billing.manage` | writes |
| `plans.create_plan` | `plans.plans.manage` | writes |
| `plans.get_entitlements` | — | read-only |
| `plans.get_plan` | `plans.plans.read` | read-only |
| `plans.get_provisioning` | `plans.subscriptions.read` | read-only |
| `plans.get_subscription` | `plans.subscriptions.read` | read-only |
| `plans.list_plans` | `plans.plans.read` | read-only |
| `plans.list_signups` | `plans.subscriptions.read` | read-only |
| `plans.list_usage` | `plans.subscriptions.read` | read-only |
| `plans.provision_tenant` | `plans.provision` | writes |
| `plans.retire_plan` | `plans.plans.manage` | writes |
| `plans.set_override` | `plans.subscriptions.manage` | writes |
| `plans.update_plan` | `plans.plans.manage` | writes |
| `tax.quote` | `tax.read` | read-only |
| `tax.rates` | `tax.read` | read-only |
| `webhooks.list_deliveries` | `webhooks.manage` | read-only |
| `webhooks.redeliver` | `webhooks.manage` | writes |

<!-- tools:end -->

## 4. Resources and prompts

Besides tools, `/mcp` offers **resources** (read-only documents: `resources/list`, `resources/read`) and **prompts** (templates for common jobs: `prompts/list`, `prompts/get`). Both follow permissions like tools: a resource is listed only to callers with its permission, and a prompt only to callers who may call every tool it leads to. A resource or prompt the caller may not use answers exactly like one that does not exist.

Every signed-in caller can read `autoxide://kernel/llms.txt` (the same text as `/llms.txt`) and `autoxide://kernel/openapi.json` (the REST contract). Prompts never call anything by themselves: they return messages that tell the model which tools to call, and the user's arguments come inside tags such as `<topic>…</topic>`, marked as data.

The resources and prompts of this version (`docs/mcp-tools.json` lists them too):

<!-- resources:start -->

| Resource | Permission | Type |
|---|---|---|
| `autoxide://comments/guide` | `comments.moderate` | text/markdown |
| `autoxide://content/guide` | `content.read` | text/markdown |
| `autoxide://content/schema/entry` | `content.read` | application/schema+json |
| `autoxide://kernel/modules` | `kernel.modules.read` | application/json |

<!-- resources:end -->

<!-- prompts:start -->

| Prompt | Arguments | Leads to |
|---|---|---|
| `comments.moderate_queue` | `limit` | `comments.queue`, `comments.moderate` |
| `content.draft_post` | `topic` (required), `locale`, `audience` | `content.create_draft` |

<!-- prompts:end -->

## 5. Let the agent find its way: `/llms.txt`

`/llms.txt` is a public page in the [llmstxt.org](https://llmstxt.org) format: where the OpenAPI document and `/mcp` are, how to authenticate, the conventions every endpoint follows, the installed modules and the MCP prompts and resources. It is generated from the code when the server starts, so it always matches this version. Links are absolute when `AUTOXIDE_HTTP_PUBLIC_URL` is set.

<!-- example: llms-txt -->
```bash
curl -sS "$API/llms.txt"
```
