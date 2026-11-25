- **Breaking:** list endpoints return `Page<T>` (`{"data": [...], "pagination": {"limit", "next_cursor",
  "total"?}}`) with `limit` / `cursor` / `order` / `count`, instead of every item at once. Follow
  `pagination.next_cursor` until it is `null` to get the whole list. This release: `GET
  /api/v1/support/routing-rules` (order: as tried, `position` then creation; `order=desc` reverses),
  `GET /api/v1/support/tickets/{id}/reply-drafts` (newest first), `GET /api/v1/support/macros` (by name;
  no longer capped at 500).
