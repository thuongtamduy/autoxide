- `cargo xtask new-module --external` now also writes a `README.md` + `README.vi.md`, an `AGENTS.md`
  with the rules for AI agents (`CLAUDE.md` links to it) and a CI workflow that runs `check.sh` on
  SQLite and PostgreSQL; `--core-path <checkout>` depends on a local checkout of Autoxide instead of
  the release tag. The gate now builds and tests a generated workspace.
