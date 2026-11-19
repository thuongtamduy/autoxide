- New module `conversations` (`/api/v1/conversations`): customers chat with a shop or a marketplace
  store about a product or an order. Messages are append-only, resending with the same `client_id`
  never duplicates, and authors may recall a message within 2 minutes. Staff share an inbox with
  unread counts, assignment, read cursors, AI drafts, hand-over to a support ticket, blocking and
  reports of abuse. Simple questions get automatic answers through support's approved answers;
  people offline get one batched notice. Conversations are deleted
  `AUTOXIDE_CONVERSATIONS_RETENTION_DAYS` after the last message; deleting an account erases the
  customer's own messages. MCP tools: `conversations.list_threads`, `get_thread`, `reply`,
  `suggest_reply`.
- Event streams: an event can also reach the users it names (`stream_audience` in the module
  catalog), so customers receive chat events on `GET /api/v1/kernel/events/stream` without any
  permission.
