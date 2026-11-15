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
| `ai.status` | `ai.view` | read-only |
| `audit.get_entry` | `audit.read` | read-only |
| `audit.list_entries` | `audit.read` | read-only |
| `auth.create_api_key` | — | writes |
| `auth.create_role` | `auth.roles.manage` | writes |
| `auth.delete_role` | `auth.roles.manage` | destructive |
| `auth.get_role` | `auth.roles.read` | read-only |
| `auth.get_user_roles` | `auth.users.read` | read-only |
| `auth.list_api_keys` | — | read-only |
| `auth.list_permissions` | `auth.roles.read` | read-only |
| `auth.list_roles` | `auth.roles.read` | read-only |
| `auth.list_users` | `auth.users.read` | read-only |
| `auth.replace_role` | `auth.roles.manage` | writes |
| `auth.revoke_api_key` | — | destructive |
| `auth.set_user_roles` | `auth.roles.manage` | writes |
| `auth.update_user` | `auth.users.manage` | writes |
| `auth.whoami` | — | read-only |
| `comments.answer` | `comments.post` | writes |
| `comments.get` | `comments.read` | read-only |
| `comments.moderate` | `comments.moderate` | writes |
| `comments.queue` | `comments.read` | read-only |
| `comments.set_target_state` | `comments.manage` | writes |
| `comments.unanswered` | `comments.read` | read-only |
| `content.create_draft` | `content.write` or `content.write_own` | writes |
| `content.get` | `content.read` | read-only |
| `content.get_translation_request` | `content.read` or `content.write_own` | read-only |
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
| `kernel.current_tenant` | — | read-only |
| `kernel.get_module` | `kernel.modules.read` | read-only |
| `kernel.get_tenant` | `kernel.tenants.read` | read-only |
| `kernel.list_modules` | `kernel.modules.read` | read-only |
| `kernel.list_settings` | `kernel.settings.manage` | read-only |
| `kernel.list_tenants` | `kernel.tenants.read` | read-only |
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

<!-- tools:end -->
