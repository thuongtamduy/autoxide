- Offline sync (step 69): a module declares `Module::sync_resources` and gets a change feed
  (`GET /api/v1/<module>/<path>/changes?cursor=`) with the current record or a deletion mark,
  filtered like its list; an old cursor answers `410 sync_cursor_expired`
  (`AUTOXIDE_SYNC_RETENTION_DAYS`). A stale `If-Match` answers `412` with the current record in
  the problem's `current` and its `ETag`. `POST /api/v1/kernel/sync/batch` runs queued writes one
  by one through their REST endpoints (`GET /api/v1/kernel/sync/resources` lists what syncs).
  Module `notes` is the worked example.
