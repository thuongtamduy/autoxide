# OpenSSF badges: Scorecard and Best Practices

**English** · [Tiếng Việt](OPENSSF.vi.md)

Two independent OpenSSF programs measure how safely an open-source project is run. Autoxide uses both.

## OpenSSF Scorecard (automatic)

[Scorecard](https://scorecard.dev) scores the repository on checks such as pinned dependencies, token permissions, code
review, branch protection, signed releases and fuzzing. The workflow
[`.github/workflows/scorecard.yml`](../.github/workflows/scorecard.yml) runs it weekly and on every push to `main`,
publishes the result and uploads it to the Security tab. Nothing to do per release.

Add the badge to the README once the first run has published:

```markdown
[![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/OWNER/REPO/badge)](https://scorecard.dev/viewer/?uri=github.com/OWNER/REPO)
```

Some checks depend on repository settings that a workflow cannot change. The maintainers must set them in
**Settings → Branches** (branch protection or a ruleset on `main`): require pull requests, require the `gate (…)`
status checks, require at least one approving review, block force pushes. Turning these on is the largest single
improvement of the score.

## OpenSSF Best Practices badge (answered on the web)

The [Best Practices badge](https://www.bestpractices.dev) is a questionnaire the maintainers answer, with evidence links.
It has three levels: passing, silver and gold. Steps:

1. Sign in at bestpractices.dev with the GitHub account of a maintainer and add the repository.
2. Answer the questions. Most can be answered with a link from this repository:

   | Criterion                         | Evidence in this repository                              |
   | --------------------------------- | -------------------------------------------------------- |
   | Project website, description      | `README.md`                                              |
   | How to contribute                 | `CONTRIBUTING.md`, `GOVERNANCE.md`                       |
   | FLOSS license                     | `LICENSE-MIT`, `LICENSE-APACHE` (dual-licensed)          |
   | Documentation                     | `docs/`, the OpenAPI contract `docs/openapi.json`        |
   | HTTPS for the project site        | GitHub Pages and github.com                              |
   | Public version-controlled source  | GitHub                                                   |
   | Unique version numbers, release notes | SemVer tags, `CHANGELOG.md`                          |
   | Bug-reporting process             | `.github/ISSUE_TEMPLATE/`, `SUPPORT.md`                  |
   | Vulnerability reporting process   | `SECURITY.md` (private reporting, response times)        |
   | Working build system              | `cargo build`; reproducible build in `docs/COMPATIBILITY.md` |
   | Automated test suite              | `./gate.sh`, `.github/workflows/ci.yml`                  |
   | New functionality has tests       | Rule in `CONTRIBUTING.md`, checked in review             |
   | Warning flags / linters           | Clippy, `cargo xtask check`, zizmor, actionlint          |
   | Static analysis                   | CodeQL (`codeql.yml`), Clippy                            |
   | Dynamic analysis, fuzzing         | `fuzz/` and `.github/workflows/fuzz.yml`                 |
   | Known vulnerabilities fixed       | `cargo deny` in CI, nightly `audit.yml`, Dependabot      |
   | Secrets are not stored in git     | gitleaks in CI                                           |
   | Signed releases / provenance      | Release workflow (see `docs/RELEASING.md`)               |
   | Reproducible builds (gold)        | `docs/COMPATIBILITY.md`, "Reproducible builds"          |
   | Two-person review (gold)          | Branch protection; needs a second maintainer             |

3. Copy the badge snippet bestpractices.dev gives you (it contains the numeric project id) into the README.

Be honest in the answers: a criterion marked "met" must be true and the evidence link must show it. Criteria that are not
yet true (for example the second reviewer for gold) stay "unmet" until they are.

## Who does what

| Step                                          | Who                          |
| --------------------------------------------- | ---------------------------- |
| Scorecard and CodeQL workflows                | Done in the repository       |
| Branch protection on `main`                   | Project owner (repo settings) |
| Enable private vulnerability reporting        | Project owner (repo settings) |
| Create the Best Practices entry, answer it    | Project owner (web)          |
| Add both badges to the README                 | Any contributor, after the above |
