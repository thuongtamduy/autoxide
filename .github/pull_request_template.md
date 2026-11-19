## What and why

<!-- What does this change do, and why is it needed? Link the issue: "Closes #…". -->

## Checklist

- [ ] The full `./gate.sh` is green (SQLite **and** PostgreSQL), not only `./gate.sh --quick`
- [ ] New code has tests, and they run on both databases
- [ ] Changed an English document? The Vietnamese one (`*.vi.md`) is changed in the same commit (and the other way round)
- [ ] Anything users would notice has a fragment in `changelog.d/` (`<name>.<section>.md`; do not edit `CHANGELOG.md`)
- [ ] Changed the API or a setting? `docs/openapi.json` / `.env.example` are regenerated
- [ ] New error `code`? It has a Vietnamese translation in the crate's `locales/vi.json`
