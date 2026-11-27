- `autoxide backup` no longer needs the database migrated first: a backup keeps the schema of its source exactly as
  it is (`backup.json` lists the migrations it holds), so you can back up right before an upgrade and the release you
  upgrade from can restore it. An older release than the source refuses the backup with a clear message. Scheduled
  backups do the same.
- `autoxide backup --all` also backs up every placement, with its own file storage, into `placements/<name>/`;
  `backup.json` of the main database is written last, so an interrupted run never looks complete.
  `autoxide restore <dir> --placement <name>` restores one into that same placement, once its database is empty again.
