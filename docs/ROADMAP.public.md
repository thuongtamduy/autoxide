# Public roadmap

A short English overview of where Autoxide is going. It deliberately does not list individual tasks: the single source
of truth is [`docs/ROADMAP.md`](ROADMAP.md), the working plan of the maintainers and their AI agents, **written in
Vietnamese** (translate it with any tool; item titles are plain technical text). Changes of direction go through
[`GOVERNANCE.md`](../GOVERNANCE.md) (RFCs in [`docs/rfcs/`](rfcs/README.md)).

## Where we are

The latest release is in the 0.x series. **Feature freeze until 1.0**: no new business modules; the work is the stable
contract, upgrades, AI-agent standards, onboarding and community standards. It is tracked as "step 71" of the roadmap, in
groups G1 to G14.

## What 1.0 means

The first release with a stability promise ([`docs/COMPATIBILITY.md`](COMPATIBILITY.md)):

- A **stable tier** (REST API and OpenAPI, error codes, configuration, migrations, MCP tools, events and webhooks, the
  CLI, the Rust API for modules, the TypeScript SDK) that breaks only in a major release, and an **experimental tier**
  that is labelled and may change in a minor one.
- **One-step upgrades and rollback**, with library major versions isolated behind kernel layers.
- **Both SQLite and PostgreSQL**, always.
- Standard sign-in for AI agents (MCP with OAuth 2.1), extension without writing Rust, and the community and supply-chain
  standards of an open-source project.
- Exit criteria: every roadmap item of step 71 done, the full gate green on both databases, an external security test
  with no high-severity finding, legal review of personal data, e-invoicing and tax, and measured quality goals.

## After 1.0

Business modules wait until after 1.0. New modules are best written **outside this repository** against the stable
contract: see [`docs/MODULE_ECOSYSTEM.md`](MODULE_ECOSYSTEM.md).

## Rules every change follows

The same rules apply to people and AI agents; the short list is in
[`CONTRIBUTING.md`](../CONTRIBUTING.md#the-rules-that-matter-most) and the full text in [`CLAUDE.md`](../CLAUDE.md).

## How to take part

- Pick a [`good first issue`](https://github.com/thuongtamduy/autoxide/labels/good%20first%20issue) or
  [`help wanted`](https://github.com/thuongtamduy/autoxide/labels/help%20wanted).
- Propose bigger changes as an [RFC](rfcs/README.md).
- Build and share a module: [`docs/MODULE_ECOSYSTEM.md`](MODULE_ECOSYSTEM.md).
- Ask questions in [Discussions](https://github.com/thuongtamduy/autoxide/discussions) ([`SUPPORT.md`](../SUPPORT.md)).
