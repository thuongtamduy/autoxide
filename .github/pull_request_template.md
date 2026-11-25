## What and why

<!-- What does this change do, and why is it needed? Link the issue: "Closes #…". -->

## Checklist

- [ ] `./gate.sh --quick` is green (SQLite). CI runs the rest, including PostgreSQL; the full `./gate.sh` is required before a release, not before each PR
- [ ] New code has tests, and they run on both databases (CI runs them on PostgreSQL; run the one you wrote on PostgreSQL locally while working)
- [ ] Changed an English document? The Vietnamese one (`*.vi.md`) is changed in the same commit (and the other way round)
- [ ] Anything users would notice has a fragment in `changelog.d/` (`<name>.<section>.md`; do not edit `CHANGELOG.md`)
- [ ] Changed the API or a setting? `cargo xtask regen` was run (`docs/openapi.json`, `docs/mcp-tools.json`, `.env.example`, SDK)
- [ ] New error `code`? It has a Vietnamese translation in the crate's `locales/vi.json`
