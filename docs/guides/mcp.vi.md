# Nối AI agent qua MCP

Tiếng Anh: [`mcp.md`](mcp.md). Ví dụ dùng `curl` với biến `API` là địa chỉ máy chủ của bạn (`export API=https://api.example.com`), và SDK TypeScript (`sdk/typescript`). Chúng chạy như test với mỗi bản build, nên khớp máy chủ của phiên bản này.

AI agent gọi Autoxide như các tool qua [Model Context Protocol](https://modelcontextprotocol.io) ở `/mcp` (Streamable HTTP). Mỗi tool chạy cùng code với endpoint REST tương ứng, cùng permission: tool không bao giờ làm được điều REST với cùng permission không làm được.

## 1. Tạo key với ít permission nhất

Cho agent một API key riêng, chỉ với thứ công việc của nó cần. `content.write_own` cho nó viết bản nháp của chính nó; việc đăng vẫn do người làm.

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

## 2. Nối client

Claude Code:

```bash
claude mcp add --transport http autoxide https://api.example.com/mcp --header "Authorization: Bearer $AUTOXIDE_AGENT_KEY"
```

Client MCP khác nhận một máy chủ Streamable HTTP kèm header, vd:

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

## 3. Agent làm được gì

`tools/list` chỉ hiện tool mà permission của key cho phép. Mô tả của mỗi tool kết thúc bằng permission cần có, và chú thích cho biết tool chỉ đọc hay phá huỷ (client hỏi người dùng trước khi gọi loại này). Tool deprecated ghi rõ trong mô tả và trong mọi kết quả (`_meta.autoxide/deprecation`), kèm tool dùng thay.

Giới hạn:

- Request bằng API key dùng chung giới hạn của key (`AUTOXIDE_HTTP_RATE_LIMIT_API_KEY_PER_MINUTE`).
- Mọi lệnh ghi vào audit log dưới tên key.
- Kết quả AI là gợi ý và bản nháp để người duyệt; agent không tự đăng được gì.
- Kết quả tool cùng hình dạng với REST: `{"data": …}`, hay một trang kèm `pagination`; lỗi mang cùng `code`.

Tool của phiên bản này, theo module (`docs/mcp-tools.json` có đủ schema đầu vào):

<!-- tools:start -->

| Tool | Permission | Loại |
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
| `kernel.add_domain` | `kernel.domains.manage` | writes |
| `kernel.current_tenant` | — | read-only |
| `kernel.get_module` | `kernel.modules.read` | read-only |
| `kernel.get_tenant` | `kernel.tenants.read` | read-only |
| `kernel.get_tenant_move` | `kernel.tenants.read` | read-only |
| `kernel.list_domains` | `kernel.domains.manage` | read-only |
| `kernel.list_modules` | `kernel.modules.read` | read-only |
| `kernel.list_settings` | `kernel.settings.manage` | read-only |
| `kernel.list_tenants` | `kernel.tenants.read` | read-only |
| `kernel.move_tenant` | `kernel.tenants.move` | destructive |
| `kernel.update_domain` | `kernel.domains.manage` | writes |
| `kernel.verify_domain` | `kernel.domains.manage` | writes |
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
| `organizations.change_unit_status` | `organizations.units.manage` | destructive |
| `organizations.create_unit` | `organizations.units.write` | writes |
| `organizations.get_unit` | `organizations.units.read` | read-only |
| `organizations.list_units` | `organizations.units.read` | read-only |
| `organizations.move_unit` | `organizations.units.write` | writes |
| `organizations.update_unit` | `organizations.units.write` | writes |

<!-- tools:end -->
