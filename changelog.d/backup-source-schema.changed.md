- `autoxide backup` no longer needs the database migrated first: a backup keeps the schema of its source exactly as
  it is (`backup.json` lists the migrations it holds), so you can back up right before an upgrade and the release you
  upgrade from can restore it. It also holds the data of modules that are turned off (it used to refuse). Scheduled
  backups do the same.
- `autoxide backup --all` also backs up every placement, with its own file storage, into `placements/<name>/`;
  `autoxide restore <dir>/placements/<name> --placement <name>` restores one into a placement whose database is empty.
