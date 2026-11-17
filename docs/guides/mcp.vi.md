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
| `compliance.add_incident_entry` | `compliance.incidents.manage` | writes |
| `compliance.documents` | `compliance.documents.read` | read-only |
| `compliance.incident_draft` | `compliance.incidents.manage` | writes |
| `compliance.incidents` | `compliance.incidents.manage` | writes |
| `compliance.kyc_profiles` | `compliance.kyc.read` | read-only |
| `compliance.legal_profiles` | `compliance.legal.read` | read-only |
| `compliance.open_incident` | `compliance.incidents.manage` | writes |
| `compliance.pending_acceptances` | — | read-only |
| `compliance.processing_register` | `compliance.privacy.read` | read-only |
| `compliance.publish_document_version` | `compliance.documents.manage` | writes |
| `compliance.purposes` | `compliance.privacy.read` | read-only |
| `compliance.record_subject_request` | `compliance.privacy.manage` | writes |
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
| `kernel.export_tenant` | `kernel.data.export` | writes |
| `kernel.get_migration_run` | `kernel.tenants.operate` | read-only |
| `kernel.get_module` | `kernel.modules.read` | read-only |
| `kernel.get_tenant` | `kernel.tenants.read` | read-only |
| `kernel.get_tenant_export` | `kernel.data.export` | read-only |
| `kernel.get_tenant_move` | `kernel.tenants.read` | read-only |
| `kernel.list_domains` | `kernel.domains.manage` | read-only |
| `kernel.list_modules` | `kernel.modules.read` | read-only |
| `kernel.list_settings` | `kernel.settings.manage` | read-only |
| `kernel.list_tenants` | `kernel.tenants.read` | read-only |
| `kernel.migrate_placements` | `kernel.tenants.operate` | writes |
| `kernel.move_tenant` | `kernel.tenants.move` | destructive |
| `kernel.navigation` | — | read-only |
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
| `notifications.list_kinds` | — | read-only |
| `notifications.list_messages` | `notifications.messages.read` | read-only |
| `notifications.list_my_inbox` | — | read-only |
| `notifications.mark_inbox_read` | — | writes |
| `notifications.send_notification` | `notifications.send` | writes |
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

<!-- tools:end -->
