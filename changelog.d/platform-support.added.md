- Supported platforms are now a stated commitment ("Supported platforms" in `docs/COMPATIBILITY.md`): PostgreSQL 16, 17
  and 18 (majors still maintained upstream), SQLite 3.45 or newer, the image for `linux/amd64` and `linux/arm64`, and the
  Linux, macOS and Windows binaries. `autoxide doctor` has a new check `database_version` that warns when the database
  runs a version outside that list or past its upstream end of life.
- Kernel: `Database::server_version()` returns the version the database server reports (PostgreSQL
  `server_version_num`, SQLite `sqlite_version()`).
