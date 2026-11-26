# Governance

**English** · [Tiếng Việt](GOVERNANCE.vi.md)

Autoxide is an open-source project run by a small group of maintainers. This document says who decides, how, and how
you can change the project.

## Roles

- **Users** use Autoxide and report problems. No permission needed.
- **Contributors** send pull requests, documentation, reviews or module ideas. See [`CONTRIBUTING.md`](CONTRIBUTING.md).
- **Maintainers** review and merge, cut releases, run the security process and keep the project on its roadmap. They
  are listed in [`.github/CODEOWNERS`](.github/CODEOWNERS). The project owner (the first maintainer) holds the final
  say on direction and on the items marked "owner's decision".

A contributor becomes a maintainer after a sustained record of good changes and reviews, when an existing maintainer
proposes it and no maintainer objects within seven days. A maintainer who is inactive for six months is moved to
"emeritus" and can return by asking. The list of maintainers is the CODEOWNERS file; a maintainer
is identified by GitHub login, which is also what exempts their pull requests from the DCO check.

## How decisions are made

1. **Everyday changes** (bug fixes, small features inside the current roadmap, documentation) need one maintainer's
   approval and a green CI. The author does not approve their own change.
2. **Changes to the stable contract tier** (public API of the kernel, testkit, a module's `api`, REST/MCP contract,
   settings, migrations) or to security, money and tax code need an independent review from a second person and follow
   [`docs/COMPATIBILITY.md`](docs/COMPATIBILITY.md).
3. **Significant changes** (new core capability, change of a convention, a library swap, anything that is expensive to
   undo) go through the RFC process below before code is written.
4. Decisions are made by **lazy consensus**: a proposal stands when no maintainer objects within the review window
   (seven days for an RFC). If maintainers disagree and cannot settle it, the project owner decides and records the reason.
5. The project's long-lived decisions are written in `docs/decisions/` (one file each) and are not reversed silently;
   a reversal is a new decision that names the one it replaces.

## Proposing a change: the RFC process

An RFC (request for comments) is a short design document that you discuss before building.

1. Discuss the idea first in an issue or a GitHub Discussion, to avoid writing an RFC for something already planned.
2. Copy [`docs/rfcs/0000-template.md`](docs/rfcs/0000-template.md) to `docs/rfcs/NNNN-short-name.md` (use the next free
   number) and fill it in. RFCs are written in English.
3. Open a pull request titled `rfc: <short name>`. The pull request is the discussion.
4. Maintainers and the community comment. The author updates the text. After the seven-day window with no
   unresolved objection, a maintainer merges it with status `accepted`, or closes it with status `rejected` and the reason.
5. Accepted RFCs are implemented in later pull requests that link the RFC. An RFC that changes a lasting rule also
   adds a decision file in `docs/decisions/`.

See [`docs/rfcs/README.md`](docs/rfcs/README.md) for what needs an RFC and what does not.

## Issue and pull request labels

| Label                       | Meaning                                                                      |
| --------------------------- | ---------------------------------------------------------------------------- |
| `good first issue`          | Small, well described, a maintainer will help; a good first contribution     |
| `help wanted`               | We want a contribution; any size, not necessarily easy                       |
| `bug`                       | Something does not work as documented                                        |
| `enhancement`               | A new feature or improvement                                                 |
| `documentation`             | Docs only                                                                    |
| `question`                  | A usage question (prefer GitHub Discussions)                                 |
| `needs-reproduction`        | We cannot reproduce it yet; the reporter is asked for steps                  |
| `needs-rfc`                 | Too big for a pull request alone; write an RFC first                         |
| `breaking-change`           | Changes the stable contract tier; needs a deprecation path                   |
| `security`                  | Handled privately first; see [`SECURITY.md`](SECURITY.md)                    |
| `module`, `module:<name>`   | Concerns a module (and which one)                                            |
| `db:sqlite`, `db:postgres`  | Only reproduces on one database                                              |
| `wontfix`, `duplicate`      | Closed with a reason                                                         |

A maintainer or the owner creates these labels once in the repository settings (the labels cannot be created by a
file in the repository).

## Code of conduct and conflicts

Everyone follows the [code of conduct](CODE_OF_CONDUCT.md). Conduct reports go to the contact in that document, not to a
public issue. Disagreements about technical direction follow the decision rules above.

## Changing this document

Changes to this document are a pull request that needs approval from the project owner. Files that decide who is exempt from
checks or how releases are made (`.github/`, `scripts/`) are listed in `.github/CODEOWNERS`; the repository settings must
require a **code owner review** on `main` for that to bind (a setting of the project owner, see `docs/OPENSSF.md`).
