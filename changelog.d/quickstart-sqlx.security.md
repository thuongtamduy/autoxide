- The bundled SQLite is now built from `libsqlite3-sys` 0.40 (through `sqlx` 0.9.1), which fixes
  RUSTSEC-2026-0459; the exception for it is gone from `deny.toml`.
