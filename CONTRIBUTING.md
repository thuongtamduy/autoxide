# Contributing to Autoxide

**English** · [Tiếng Việt](CONTRIBUTING.vi.md)

Thank you for helping. This guide is for people; if you work with an AI coding agent, point
it at [`CLAUDE.md`](CLAUDE.md) (also linked as `AGENTS.md`), which holds the full rules the
agent follows. The rules below are the same, only shorter. Everyone taking part follows the
[code of conduct](CODE_OF_CONDUCT.md).

## Before you start

- **Bugs**: open an issue with the version, the database (SQLite or PostgreSQL) and the
  steps that reproduce it.
- **Security problems**: do not open an issue; follow [`SECURITY.md`](SECURITY.md).
- **New features**: open an issue first and describe the problem you want to solve. The
  core stays neutral about business domains (no orders, stock or patients in the core —
  those belong in modules), and the order of work follows
  [`docs/ROADMAP.md`](docs/ROADMAP.md), so a short discussion saves you from writing code
  that cannot be merged.

## Set up your machine

You need:

- [Rust](https://rustup.rs) — `rustup` installs the exact toolchain pinned in
  `rust-toolchain.toml` the first time you run `cargo`;
- a C compiler (SQLite is compiled from source);
- [Docker](https://docs.docker.com/get-docker/), for the PostgreSQL and S3 tests of the full
  gate;
- [`cargo-deny`](https://github.com/EmbarkStudios/cargo-deny):
  `cargo install cargo-deny --locked`;
- optionally [`gitleaks`](https://github.com/gitleaks/gitleaks) (the gate runs it in Docker
  otherwise).

On a fresh Linux x86_64 machine (for example a cloud agent session) run `scripts/cloud-setup.sh`
instead of installing by hand: it installs the pinned toolchain and `cargo-deny`, `gitleaks` and
`cargo-nextest` from prebuilt releases, checking pinned SHA-256 checksums. It is safe to run again.

Then:

```bash
git clone https://github.com/thuongtamduy/autoxide.git autoxide
cd autoxide
git config core.hooksPath .githooks
cargo test --workspace
```

The Git hooks check formatting and scan staged changes for secrets before each commit.

## Check your change: two gates

The gate is the definition of "done". There are two levels:

```bash
./gate.sh --quick
```

While you work, before each commit: formatting, lints, boundary checks, dependencies,
secrets, and the tests of the crates you changed (and of the crates that depend on them),
on SQLite only. It needs no Docker for the tests and usually takes a minute or two.

```bash
./gate.sh
```

Before you open a pull request: everything above, and every test on **both SQLite and
PostgreSQL** (the gate starts throwaway Docker containers). A change is merged only when
this gate is green.

When the gate is red, fix the code. Do not loosen the gate, turn off a lint or skip a test.

A change that only touches files listed in [`scripts/rust-free-paths.txt`](scripts/rust-free-paths.txt)
(documentation, the TypeScript SDK, changelog fragments, decisions) cannot change any Rust result: `--quick` skips
Clippy for it, and on a pull request CI skips the Rust build and tests (the jobs still report success). Anything not
in that list runs everything.

## The rules that matter most

- **Both databases.** Everything that touches the database — migrations, queries,
  transactions, indexes, tests — must work on SQLite and PostgreSQL. A feature that works
  on one only is not finished. Do not use types, functions or syntax only one of them has.
- **Module boundaries.** Each module is its own crate under `modules/` and owns its tables
  (named `<module>_…`). A module never reads, writes or joins another module's tables and
  never has foreign keys to them; it calls other modules only through the traits they
  publish in `pub mod api`. `cargo xtask check` (part of the gate) enforces this.
- **Logic in services.** HTTP handlers and MCP tools only take input, call a service and
  return the response; REST and MCP share the same service.
- **Errors have a stable `code`.** Every error returned to clients is an
  `autoxide_kernel::error::Error` with a `snake_case` code (module errors start with
  `<module>.`) and a message that says what to fix. The code is a contract with clients:
  changing it is a breaking change. Each new code needs a Vietnamese translation in the
  crate's `locales/vi.json`.
- **The API contract stays in sync.** Every route is documented in OpenAPI. When you change
  the API or a setting, run `cargo xtask regen`: it regenerates every generated file in one go
  (`docs/openapi.json`, `docs/mcp-tools.json`, `.env.example`, the TypeScript SDK schema). The
  the gate on your machine checks them early (`cargo xtask regen --check`) and prints this command when one is stale (this step runs only locally; on CI the tests that keep the documents in sync with the code check them). Never change the behaviour
  of an existing `/api/v1` endpoint in a breaking way; add a new one and deprecate the old.
- **Tests.** New code comes with tests. Tests that touch the database use
  `autoxide_testkit::TestDatabase`, which gives each test an empty database on whichever
  of SQLite or PostgreSQL the gate is running.
- **No `unsafe`, no `unwrap` / `expect` / `panic!` outside tests.** Return an error instead.
- **No secrets in Git.** Only `.env.example` is committed, never `.env`.
- **Documentation in two languages.** User-facing documents (`README`, guides in `docs/`,
  this file) have an English version and a Vietnamese one (`*.vi.md`) with the same
  content: change both in the same commit. Internal documents (`CLAUDE.md`, `STACK.md`,
  `docs/ROADMAP.md`, `docs/DECISIONS.md`) and code comments may be in Vietnamese.

The full list, with the reasons behind each rule, is in [`CLAUDE.md`](CLAUDE.md); past
decisions are indexed by topic in `docs/decisions/INDEX.md`, which is generated and not committed:
run `cargo xtask decisions` to create it.

## Commits and pull requests

- Keep a pull request to one change. Split unrelated changes.
- Write the commit subject as a short summary of what changed (English or Vietnamese), and
  explain *why* in the body when it is not obvious.
- For anything a user of Autoxide would notice, add a small file to
  [`changelog.d/`](changelog.d/README.md) instead of editing `CHANGELOG.md`: name it
  `<name>.<section>.md` (`added`, `changed`, `deprecated`, `removed`, `fixed` or `security`) and write
  the English bullet points as they will appear in the changelog. They are folded into `CHANGELOG.md`
  at release time, so parallel pull requests never conflict there.
- In the pull request, say what you changed, why, and confirm that `./gate.sh --quick` is
  green (CI runs the rest, including PostgreSQL; the full `./gate.sh` is required before a release).

## License of your contribution

Autoxide is licensed under either of the Apache License, Version 2.0 or the MIT license,
at the user's option. Unless you explicitly state otherwise, any contribution you
intentionally submit for inclusion in Autoxide, as defined in the Apache-2.0 license, is
dual licensed as above, without any additional terms or conditions.
