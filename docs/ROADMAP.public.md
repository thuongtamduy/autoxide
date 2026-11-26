# Public roadmap

This is the public summary of where Autoxide is going. The detailed, step-by-step working plan used by the maintainers and
their AI agents is [`docs/ROADMAP.md`](ROADMAP.md) (internal, in Vietnamese); this page is the English overview for
everyone else. Changes of direction go through [`GOVERNANCE.md`](../GOVERNANCE.md) (RFCs in [`docs/rfcs/`](rfcs/README.md)).

Status on 2026-11-26: the latest release is `0.19.x`. **Feature freeze until 1.0**: no new business modules; every effort
goes into the contract, the upgrade path, AI-agent standards, onboarding and community standards.

## What 1.0 means

Autoxide 1.0 is the first release with a stability promise ([`docs/COMPATIBILITY.md`](COMPATIBILITY.md)):

- **Two contract tiers.** A *stable* tier (the core: REST API and OpenAPI, error codes, configuration, migrations, MCP
  tools, events and webhooks, the CLI, the Rust API for modules, the TypeScript SDK) follows semantic versioning: no
  breaking change before 2.0, deprecations announced first. An *experimental* tier (newer or niche modules) is clearly
  labelled and may change in a minor release.
- **Upgrades are one step.** `autoxide upgrade` checks compatibility, backs up, migrates every tenant database and
  runs `doctor`; `autoxide rollback` goes back. Library major versions are isolated behind kernel layers, so a
  dependency upgrade does not force an Autoxide major version.
- **Both databases, always.** Every feature is green on SQLite and PostgreSQL.
- **Exit criteria.** Every item of step 71 below is done; the full gate is green on both databases; an external
  security test finds nothing of high severity or above; the legal review of personal data, e-invoicing and tax is done;
  and the measured goals under "Quality" are met.

## Step 71: the road to 1.0

| Group | Theme | What it delivers |
| ----- | ----- | ---------------- |
| G1  | Scope and contract | Stable and experimental tiers; one way to declare a module; `#[non_exhaustive]` public types; the kernel split into crates; runtime tenant isolation; one tenant-settings system |
| G2  | Libraries do not hold us back | Kernel layers over ORM, web framework, OpenAPI and validation libraries; modules use libraries only through them; a tool and rhythm for major upgrades |
| G3  | One-button upgrade | `autoxide upgrade` / `rollback`; backups; tenant databases migrated in parallel; compose and Kubernetes use the same command |
| G4  | AI-agent standards | MCP sign-in with OAuth 2.1 (RFC 9728 metadata, PKCE, client metadata documents); scoped, expiring, revocable delegation; human approval for destructive or money-touching tools; prompt-injection marking; a complete MCP tool contract; discovery files; agent-driven nightly evals; a Python SDK |
| G5  | Extending without Rust | WebAssembly plugins (design first), plus no-code paths: automations, custom fields, webhook recipes |
| G6  | Every skill level | `autoxide demo`, one-line `docker run`, prebuilt binaries, devcontainer, a "first module in 50 lines" guide, a standalone CLI, guardrails for AI coding assistants |
| G7  | Missing protocol standards | `RateLimit` headers and 429 documentation, ETag details, OpenAPI 3.2 and webhooks, OpenTelemetry conventions and OTLP, breached-password checks (NIST SP 800-63B-4), HSTS, a distroless image |
| G8  | Open-source community standards | CodeQL, OpenSSF Scorecard and Best Practices, DCO, governance, RFCs, SBOM, reproducible builds, MSRV CI, module ecosystem conventions, REUSE, fuzzing |
| G9  | Quality as process | Independent review for sensitive changes, mutation testing, property tests for money, tax and permissions, weekly load tests, per-release metrics |
| G10 | Lighter agent rules | A shorter `CLAUDE.md`; finished roadmap items archived |
| G11 | Reference application | A small shop built outside this repository, as a stranger would |
| G12 | People outside the code | Legal review; external security test |
| G13 | Release readiness | Versioned event, webhook and MCP contracts; a release-candidate period; long-term support policy; versioned documentation site; multi-server tests and capacity guide; threat model; recovery objectives; third-party license list |

Also before 1.0: the open items of step 54 (fine-grained authorization) and step 52 (admin experience).

### Quality goals checked at each release

- Download to first successful request: under 5 minutes.
- A newcomer finishes a first module: under 30 minutes.
- Upgrade tests in CI: 100% success, including multi-tenant setups.
- Real agents complete the sample tasks: above 90%.
- p95 latency of hot paths does not regress between two releases.

## After 1.0

Business modules wait until after 1.0 (commerce, content, bookings and others already exist in the repository, labelled by
tier). New modules are best written **outside this repository** against the stable contract:
see [`docs/MODULE_ECOSYSTEM.md`](MODULE_ECOSYSTEM.md).

## Rules every change follows

The same rules apply to people and AI agents; the short list is in [`CONTRIBUTING.md`](../CONTRIBUTING.md#the-rules-that-matter-most):

1. Everything works on **both SQLite and PostgreSQL**; tests run on both.
2. The core stays neutral about business domains; modules own their tables and talk through published traits and events.
3. Logic lives in services; REST and MCP share the same service; every route is documented in OpenAPI.
4. Errors carry a stable `code`; user-facing text goes through i18n keys.
5. No `unsafe`, no `unwrap` outside tests, no secrets in Git.
6. Every user-visible change has a changelog fragment; user-facing documents exist in English and Vietnamese.
7. Money is integers plus a currency code; IDs are UUIDv7 generated in the application; time is stored in UTC.
8. The gate (`./gate.sh --quick`, then CI) is the definition of done.

## How to take part

- Pick a [`good first issue`](https://github.com/thuongtamduy/autoxide/labels/good%20first%20issue) or
  [`help wanted`](https://github.com/thuongtamduy/autoxide/labels/help%20wanted).
- Propose bigger changes as an [RFC](rfcs/README.md).
- Build and share a module: [`docs/MODULE_ECOSYSTEM.md`](MODULE_ECOSYSTEM.md).
- Ask questions in [Discussions](https://github.com/thuongtamduy/autoxide/discussions) ([`SUPPORT.md`](../SUPPORT.md)).
