- Lists check the caller's permission before their paging parameters: a caller without the permission now gets
  403 `permission_denied` instead of 422 for a bad `limit` / `cursor` / `order` on `GET /api/v1/media`,
  `/media/folders`, `/media/tags`, `/content/entries`, `/content/entries/{id}/schedules`, `/content/menus`,
  `/content/placements`, `/content/templates`, `/bookings/exceptions` and
  `/notifications/templates/{kind}/{channel}/{locale}/versions` (more lists follow).
