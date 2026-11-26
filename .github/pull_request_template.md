## What and why

<!-- What does this change do, and why is it needed? Link the issue: "Closes #…". -->

## Independent review result

<!-- Required when the change touches `crates/kernel`, authentication / authorization / multi-tenancy, money, tax,
documents (invoices, receipts) or a migration that deletes data (D-20261126-independent-review). The reviewer is a
different agent or person than the author, reads only the code plus the contract and invariants to keep (not the
author's conclusions), and tries to build a counter-example. Delete this section for documentation-only or small
changes that touch none of the above, and say so in one line. -->

- Does this change need an independent review? <!-- yes / no (why not) -->
- Result: <!-- passed / blocking defects remain / not reviewed yet -->
- Blocking defects found and fixed, and re-reviewed: <!-- list, or "none" -->
- Deferred to later (non-blocking): <!-- list with links, or "none" -->

## Checklist

- [ ] `./gate.sh --quick` is green (SQLite). CI runs the rest, including PostgreSQL; the full `./gate.sh` is required before a release, not before each PR
- [ ] New code has tests, and they run on both databases (CI runs them on PostgreSQL; run the one you wrote on PostgreSQL locally while working)
- [ ] Changed an English document? The Vietnamese one (`*.vi.md`) is changed in the same commit (and the other way round)
- [ ] Anything users would notice has a fragment in `changelog.d/` (`<name>.<section>.md`; do not edit `CHANGELOG.md`)
- [ ] Changed the API or a setting? `cargo xtask regen` was run (`docs/openapi.json`, `docs/mcp-tools.json`, `.env.example`, SDK)
- [ ] New error `code`? It has a Vietnamese translation in the crate's `locales/vi.json`
