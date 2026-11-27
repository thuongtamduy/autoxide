- `autoxide backup` no longer needs the database migrated first: a backup keeps the schema of its source exactly as
  it is (`backup.json` lists the migrations it holds), so you can back up right before an upgrade and the release you
  upgrade from can restore it. Scheduled backups do the same. It refuses a database with no Autoxide tables (often a
  wrong `DATABASE_URL`) instead of writing an empty backup.
- `autoxide restore` checks the whole backup before it touches the target, and refuses with a clear message a backup
  made by a newer release or holding data of modules that are turned off or missing; the target stays empty, so you
  can fix the setting and run it again.
- `autoxide backup --all` also backs up every placement, with its own file storage, into `placements/<name>/`;
  `backup.json` of the main database is written last, so an interrupted run never looks complete.
  `autoxide restore <dir> --placement <name>` restores one into that same placement, once its database is empty
  again; the placement is marked `migrating` meanwhile.
