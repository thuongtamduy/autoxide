# Compatibility policy

**English** · [Tiếng Việt](COMPATIBILITY.vi.md)

This page says what you can rely on when you upgrade Autoxide, and what may change. It applies from
version **1.0.0**. Before 1.0 (`0.x`) a minor release (`0.18` → `0.19`) may break any of the
contracts below, the HTTP API included; every break is listed under **Changed** or **Removed** in
[`CHANGELOG.md`](../CHANGELOG.md) with what to do instead. From 1.0.0 the rules below apply in
full: nothing in `/api/v1` breaks.

Autoxide uses [Semantic Versioning](https://semver.org): one version number for the server, the
Rust crates and the TypeScript SDK.

| Release        | Example         | May contain                                                         |
| -------------- | --------------- | ------------------------------------------------------------------- |
| Patch          | `1.4.0 → 1.4.1` | Bug and security fixes. No new features, no new settings to set.    |
| Minor          | `1.4.1 → 1.5.0` | New features, new endpoints, new settings, deprecations. No breaks. |
| Major          | `1.9.3 → 2.0.0` | Breaking changes, each one listed with what to do instead.          |

A "break" is any change after which a client, an installation or a module that followed the
documentation stops working without being changed.

## What is a stable contract

Everything in this section belongs to the **core** and **official** modules
([`docs/MODULES.md`](MODULES.md)). Experimental modules are excluded (see
[What is not covered](#what-is-not-covered)).

### REST API and OpenAPI

The contract is `docs/openapi.json`, served at `/api/v1/openapi.json`.

- Within `/api/v1` nothing breaks, in any release: no endpoint is removed or renamed, no required
  input is added, no response field is removed or changes type, no status code changes meaning.
- A change that would break clients is made as a **new endpoint**. The old one keeps working
  exactly as before and is declared deprecated (`Module::deprecations`): the OpenAPI document marks
  it `deprecated`, and its responses carry the `Deprecation`, `Sunset` and `Link` headers
  (RFC 9745, RFC 8594) pointing to the replacement.
- A deprecated endpoint is removed only after its **sunset date**, which is at least **six months**
  after the release that deprecated it and never in a patch release.
- Minor releases may **add**: endpoints, optional request fields, query parameters, response fields,
  headers, and new values in enums of responses. Clients must ignore response fields they do not
  know and treat an unknown enum value as "other".
- A new `/api/v2` would only appear in a major release, and `/api/v1` would keep running alongside it
  for at least one more major release.
- `cargo xtask api-diff` checks every change against the last release in the gate, and fails on a
  break.

### Error codes

The `code` field of an error (`notes.title_taken`, `validation_failed`…) is a contract for programs.

- A code is never renamed, and never reused with another meaning.
- A request that failed with a given code fails with the same code after a minor release.
- New codes may be added in a minor release, for new failures. Clients must handle codes they do
  not know as the HTTP status says.
- The `message` text, its translations and `errors[].detail` are for people and may change in any
  release.

### Configuration

Environment variables (`.env.example` lists all of them), their meaning and their default values.

- A setting is never removed or renamed in a minor release. When a name has to change, the new name
  is added and the **old name keeps working**, with a warning in the startup log that names the new
  one, for at least one minor release and at least six months; it goes away only in a major release.
- A default value that changes behaviour (for example a stricter limit) changes only in a major
  release, unless it fixes a security problem; then the changelog says so under **Security**.
- New settings always have a default, so an existing `.env` keeps working.
- The same rules apply to `AUTOXIDE_MODULES`: a module never moves from official to experimental
  (which would turn it off by default) within a major version.

### Database and migrations

- A newer release always reads the data of an older one: start the new binary (or run
  `autoxide migrate up`) and migrations bring the database forward. Upgrading from **any 1.x** to
  any later 1.x works directly, without stopping at each minor release. The CI tests this upgrade
  from the previous release on both SQLite and PostgreSQL.
- A migration that was released is never edited; changes come as new migrations.
- Every migration has a `down` step, so you can go back to the previous release: with the **new**
  binary, run `autoxide migrate down <module> --steps <n>` for each migration it added (the changelog
  and `autoxide migrate status` show which), then start the old binary. Take a backup first; going down may drop data that only the new release stores.
- SQLite and PostgreSQL are both supported in every release. Support for a PostgreSQL major version
  that its upstream project no longer maintains may end in a minor release, announced in the
  changelog one minor release ahead.
- The tables themselves are **not** a contract: they belong to their module and may change shape in
  any release. Read and write data through the API, not with SQL.

### MCP tools

The tool list (`docs/mcp-tools.json`, served by the MCP endpoint) follows the REST rules: a tool keeps
its name, accepts every input it accepted and returns every field it returned. A tool that has to
change is replaced by a new tool and deprecated with a sunset date (`Deprecation::tool`), at least six
months ahead. A tool never needs a broader permission than before.

### Events and webhooks

Event names and the fields of their payloads (what modules receive and what webhooks deliver) may
gain fields in a minor release, but never lose or retype one. An event that has to change shape is
published under a new name, and the old one keeps being published until the next major release.

### Command line

The `autoxide` commands and options that the documentation describes, their exit codes, and the
output of `--json` options follow the same rules as configuration: added in minor releases, removed
or changed only in a major release. Text printed for people may change.

### Rust API for modules

For people who write modules in their own repository ([guide](guides/writing-a-module.md)), the
contract is the public Rust API of:

- `autoxide-kernel` and `autoxide-testkit`;
- `autoxide::Autoxide`, the only public item of the `autoxide` crate;
- `pub mod api` and `pub fn module(…)` of every core and official module crate.

Rules, as in [Cargo's SemVer guide](https://doc.rust-lang.org/cargo/reference/semver.html):

- A breaking change (removing or renaming a public item, changing a function signature, adding a
  required trait method without a default, …) happens only in a **major** release.
- Before it is removed, an item is marked `#[deprecated]` with a note naming the replacement, for at
  least one minor release.
- New features for modules arrive as new builder functions of `Manifest` or new registration functions
  of `StartContext` (see [Declaring a module](MODULE_GUIDE.md#declaring-a-module)), so existing modules
  compile unchanged.
- `cargo-semver-checks` compares these crates with the previous release on every pull request that
  touches them and on every release pull request; from 1.0 a break fails the check.
- A new major version of a third-party library does **not** force a major release of Autoxide, and
  does not force modules to change their code. Modules reach the database, HTTP, OpenAPI, input
  checks, migrations and time through the kernel's own layers (`autoxide_kernel::db`, `http`,
  `migration`, `time`…), and the stable public API of the kernel and the test kit carries no type of
  the libraries under those layers (`sea-orm`, `sea-query`, `sqlx`, `axum`, `http`, `tower`, `utoipa`,
  `validator`, `jiff`, `async-trait`). When one of them moves to a new major version, only the kernel
  changes. Only the small set of *vocabulary* libraries (`serde`, `serde_json`, `uuid`, `tokio`,
  `tracing`, `futures-core`, `bytes`) may appear in the contract; a new major version of one of those
  (rare: `serde` 2) is the only library change that is a breaking change. The table is in
  [`docs/MODULE_GUIDE.md`](MODULE_GUIDE.md#stability-of-the-module-contract).
- Until 1.0, modules are being moved onto those layers and the old paths still exist; `cargo xtask
  check` (and `check-module` for a module in its own repository) stops a module from depending
  directly on a library the kernel wraps, except the modules still listed as waiting to move. 1.0 is
  released only once every module has moved.
- The minimum supported Rust version follows recent stable Rust (`rust-toolchain.toml`). Raising it
  happens only in a minor release, never in a patch release, and is listed in the changelog.

#### Contract tiers of the Rust API

The public Rust API has three tiers (D-20261126-contract-tiers). Only the first is covered by the rules above.

| Tier | Where | Promise |
| ---- | ----- | ------- |
| **Stable** | `autoxide_kernel::<module>`, `autoxide_testkit`, `autoxide::Autoxide`, `api` of core and official modules | Semantic Versioning from 1.0: breaks only in a major release, `#[deprecated]` for at least one minor release first |
| **Experimental** | `autoxide_kernel::unstable::<module>`, behind the kernel Cargo feature `unstable` (off by default) | May change in any minor release; every change is in the changelog. A module that uses it turns the feature on, so it is plain what it relies on |
| **Internal** | The `autoxide-host` crate: what only the `autoxide` binary uses (logging setup, the Prometheus exporter and `/metrics`, trace export; backup, transfer and the kernel routes join it once the kernel itself no longer calls them, D-20261128-host-crate) | None: modules may not depend on it |

The experimental tier holds the contracts of provider connectors (`einvoice`, `zns`, `voice`, `sms`, `push`,
`payment`), features whose shape is still settling (`blocks`, `analytics`, `report`, `automation`,
`custom_fields`, `sync`, the operations helpers such as `find`, `suggest`, `summary`, `work`, `tags`, `bulk`…)
and the advanced multi-tenant parts (`tenancy::{sandbox, distribution, moves, domains, site, link_token}`). The
full list is in [`docs/research/kernel-contract-1-0.md`](research/kernel-contract-1-0.md#21-phân-loại-70-mô-đun-hiện-có)
(§2.1); moving an item to the stable tier is a recorded decision. In a [`Manifest`](MODULE_GUIDE.md#declaring-a-module)
the experimental concepts are declared through `Manifest::unstable`.

Before 1.0 these items still live at their current paths; they move to `unstable::` (and `bench_seed` to the
testkit) in one release before 1.0, listed under **Breaking (Rust)** in the changelog. Until then, treat the items
in the list above as experimental wherever they are.

The tier of a **module** (`official` / `experimental`, [`docs/MODULES.md`](MODULES.md)) is separate from the tier of
the kernel API: an official module may use experimental kernel items inside, but its `api` never exposes them.

Public enums and structs of the stable tier that are expected to grow are `#[non_exhaustive]` (build structs with
their constructor, add a `_ =>` arm when matching enums); adding a variant or a field to them is not a break. Today
this is done for the module contract (`Stability`, `SelectionError`, `InactiveModule`, `DormantModule`; `Manifest`
has no public fields) and for some operations types; the rest of the stable tier gets it, with a check in
`cargo xtask check`, before 1.0.

### TypeScript SDK

The SDK (`sdk/typescript`) is generated from `docs/openapi.json` and carries the same version. It
follows the REST rules: a minor release adds methods and optional fields; a method or type disappears
only in a major release. Deprecated endpoints stay in the SDK until their sunset date.

## What is not covered

These may change in any release, including patch releases where it fixes a bug:

- **Experimental modules** (tier `experimental` in [`docs/MODULES.md`](MODULES.md), off by default):
  their endpoints, tools, events, settings and Rust API may change between minor releases, breaking
  changes included. Every change is in the changelog, and moving their data to a new release is
  described in the upgrade notes when it needs a manual step.
- Anything marked **experimental** elsewhere: OpenAPI operations with `"x-stability":
  "experimental"`, MCP tools with `"stability": "experimental"`, and features whose documentation
  says "experimental".
- The experimental and internal tiers of the Rust API (see [Contract tiers of the Rust
  API](#contract-tiers-of-the-rust-api)).
- Rust items that are not public: `pub(crate)` and private items, `#[doc(hidden)]` items, and
  everything in a module crate outside `api` and `module(…)`. The sample module `notes` and the
  `xtask` crate are not part of the contract at all.
- Log lines: their format, messages and fields. Metric names and labels follow the changelog but
  are not frozen.
- Human-readable text: error `message`s, translations, descriptions in OpenAPI and MCP, CLI output
  without `--json`.
- Database tables, columns and indexes (see above), the order of items where the API does not
  promise one, and performance figures.

## Supported platforms

What the project builds, tests and fixes bugs for. Outside this list Autoxide may work, but the CI does not test it
and a problem that only shows there may not be fixed. `autoxide doctor` reports the database version it finds
(check `database_version`) and warns when it is outside the list; the server still starts.

### Databases

| Database   | Supported                                            | Tested by the CI                                   |
| ---------- | ---------------------------------------------------- | -------------------------------------------------- |
| PostgreSQL | 16, 17, 18 — the majors the PostgreSQL project still maintains, from 16 up | 16 on every pull request; 16 and 18 on `main` and before every release |
| SQLite     | 3.45 or newer                                        | the SQLite bundled in the binary (3.54)           |

- **PostgreSQL policy.** A new PostgreSQL major is added in a minor release once the full gate passes on it, and the
  CI then tests the oldest and the newest supported major. A major is dropped only after the
  [PostgreSQL project](https://www.postgresql.org/support/versioning/) stops maintaining it (PostgreSQL 16: November
  2028), announced in the changelog one minor release ahead; `doctor` warns from that date. Upgrading PostgreSQL itself
  (`pg_upgrade`, dump and restore) needs nothing from Autoxide.
- **SQLite.** The official binaries and the image bundle SQLite, so the version does not depend on the operating
  system. Only a build that you link to the system SQLite can be older; such a build is not tested.
- Managed PostgreSQL services (and PostgreSQL-compatible proxies such as PgBouncer in session mode) work when they speak
  the PostgreSQL protocol of a supported major; products that only resemble PostgreSQL are not supported.

### Operating systems and CPU architectures

| Artifact                        | Platforms                                                                    |
| ------------------------------- | ---------------------------------------------------------------------------- |
| Docker image                    | `linux/amd64`, `linux/arm64` (Debian 13 base, distroless)                    |
| Linux binary                    | `x86_64` and `aarch64` (`*-unknown-linux-gnu`), glibc 2.41 or newer (Debian 13, Ubuntu 25.04 and later) |
| macOS binary                    | Apple silicon (`aarch64-apple-darwin`) and Intel (`x86_64-apple-darwin`), macOS 11 or newer |
| Windows binary                  | `x86_64-pc-windows-msvc`, Windows 10 / Server 2016 or newer; not signed       |
| Build from source               | any target that the [MSRV](#minimum-supported-rust-version-msrv) supports at Rust tier 1 or 2 with `std` |

- Production servers: the image, or the Linux binary. The macOS and Windows binaries are for development and trying
  Autoxide; the release CI builds them, but the test suite runs on Linux.
- On an older Linux (glibc below 2.41) use the image, or build from source on that system.
- A platform is removed only in a minor release announced in the changelog one minor release ahead.

## Building from source

### Minimum supported Rust version (MSRV)

The MSRV is the `rust-version` of the workspace `Cargo.toml` (today the release before the pinned toolchain in
`rust-toolchain.toml`). Policy:

- The repository builds and its library crates compile with the MSRV; CI has a job that builds the workspace with
  exactly that version, so a newer language feature cannot slip in unnoticed.
- The pinned toolchain (`rust-toolchain.toml`, used by developers, CI and the Docker image) is newer than the MSRV and
  moves forward with stable Rust; the MSRV is raised only when a dependency or a feature needs it, never in a patch
  release, and the raise is listed in the changelog.
- Modules in other repositories need at least the MSRV. The external-module example uses the same `rust-version`
  (checked by `cargo xtask check`).

### Cargo features

The default build has every feature a production server needs. These optional features exist:

| Crate                | Feature            | What it does                                                        | In releases | Contract |
| -------------------- | ------------------ | ------------------------------------------------------------------- | ----------- | -------- |
| `autoxide`           | `avif`             | AVIF thumbnails (`AUTOXIDE_MEDIA_AVIF`); pulls the `rav1e` encoder   | Off         | Stable   |
| `autoxide`           | `bench-seed`       | `task <module>.bench_seed` generates sample data for measurements    | Off         | None     |
| `autoxide-kernel`    | `permission-audit` | Test-only: builds routes without the permission layer for audits     | Off         | None     |
| module crates        | `testing`          | Test helpers a module shares with `autoxide-testkit`                 | Off         | None     |
| module crates        | `bench-seed`, `avif` | Forwarded from the `autoxide` features above                       | Off         | As above |

"None" means not part of the compatibility promise: never enable these in an application. Features are additive; a
new feature never changes the default build.

### Reproducible builds

The release image is built with the aim that the same source gives the same binary. This is a goal: no automated test builds
twice and compares yet, so do not rely on byte-for-byte identity. What is in place:

- `Cargo.lock` is committed and every release build uses `--locked`; the toolchain is pinned (`rust-toolchain.toml`, the
  Docker `RUST_VERSION`), and the base images are pinned by digest.
- The Dockerfile builds in a fixed directory and passes `--remap-path-prefix` so no build path ends up in the binary,
  and `SOURCE_DATE_EPOCH` (the commit time of the release) is used for every timestamp, including the release archives
  (sorted, with fixed owner and modification time).
- To compare a release with your own build: rebuild from the tag with the same Dockerfile, copy
  `/usr/local/bin/autoxide` out of your image, unpack the release archive (`tar -xzf autoxide-X.Y.Z-<target>.tar.gz`) and
  compare `sha256sum` of the two `autoxide` files. (`SHA256SUMS` lists the archives, not the binary inside.) A difference
  is not proof of tampering: a different base image digest or target architecture also changes the binary.
  [`docs/VERIFYING.md`](VERIFYING.md) shows how to verify signatures and attestations, which is the supported check.

### Software bill of materials (SBOM)

Every release carries two lists of what is inside:

- **Rust crates in the binary.** The binary is built with `cargo auditable`, which embeds the exact dependency list
  in the executable. Tools such as `cargo audit bin autoxide`, Syft, Trivy or Grype read it, so a vulnerability scanner
  sees the Rust crates even in a stripped binary.
- **CycloneDX file.** `autoxide.cdx.json` (CycloneDX 1.5, generated from `Cargo.lock` by `cargo cyclonedx`) is attached to the
  GitHub Release and attested; the image also carries an SPDX SBOM (`sbom.spdx.json`).

## Support

From 1.0 each major version has long-term support:

- The latest minor release gets every fix; the previous minor release gets security fixes for **3 months** after the next
  minor release; when a new major version comes out, the last minor release of the previous major version gets
  security fixes for **6 months**. Older releases are not patched. The table is in
  [`SECURITY.md`](../SECURITY.md#supported-versions).
- **Backporting.** A fix is made on `main` first. For a supported older line, a maintainer cherry-picks it onto a
  `release/X.Y` branch cut from the latest `vX.Y.Z` tag, ships it as the next patch release (`X.Y.Z+1`) with only that
  fix, and runs the full gate on both databases first. A patch release never contains new features, new required
  settings or contract changes, so it is safe to install on its own.
- Compatibility between versions of the **database** (which version you can upgrade from, manual steps) is stated in
  the changelog of each release.
How releases are made is in [`docs/RELEASING.md`](RELEASING.md) (in Vietnamese).
