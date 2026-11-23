# Security policy

**English** · [Tiếng Việt](SECURITY.vi.md)

## Supported versions

Autoxide is a community project maintained by a small team. The periods below are what we aim
for, **not a service-level agreement**: there is no paid support and no guaranteed response time.

**From 1.0** (long-term support of each major version):

- The **latest minor release** of the current major version (for example `1.5.x` while `1.5` is the
  newest) gets every bug fix and security fix.
- The **previous minor release** gets security fixes for **3 months** after the next minor release
  comes out (for example `1.4.x` until three months after `1.5.0`), so you have time to upgrade.
- When a new major version comes out, the last minor release of the previous major version gets
  security fixes for **6 months** after the new major release.
- Older releases are not patched.

| Version                                        | Security fixes                          |
| ---------------------------------------------- | --------------------------------------- |
| Latest `1.x` minor                             | Yes                                     |
| Previous `1.x` minor                           | For 3 months after the next minor       |
| Last minor of the previous major (after `2.0`) | For 6 months after the new major        |
| Older releases (and `0.x` once 1.0 is out)    | No                                      |

**Before 1.0**, while Autoxide is in its `0.x` series, security fixes go into the latest minor
release only; older `0.x` releases are not patched.

Fixes ship as patch releases (`1.5.2` → `1.5.3`), which change nothing else, so they are safe to
install right away ([`docs/COMPATIBILITY.md`](docs/COMPATIBILITY.md) says what a patch release may
contain). Update to the latest release of your line before reporting, and say in your report which
version you tested. The `CHANGELOG.md` lists every release.

## Security reviews

Before 1.0, the maintainers reviewed the whole code base for security problems in November 2026.
It was an **internal review carried out with AI assistance, not an audit by an independent third
party**. The scope, the method, every finding with its severity and fix, and the parts not yet
covered are in [`docs/security/review-2026-11.md`](docs/security/review-2026-11.md).

## Reporting a vulnerability

**Do not open a public issue, pull request or discussion for a security problem.**

Report it privately through GitHub: open the repository's **Security** tab and choose
**Report a vulnerability** (GitHub's private vulnerability reporting). Only the maintainers
see the report, and it lets us work on a fix and an advisory with you before anything is
public.

A useful report has:

- the version (or commit) and the database (SQLite or PostgreSQL) you tested;
- the configuration that matters, with secrets removed;
- the steps or requests that reproduce the problem, and what an attacker gains;
- whether you plan to publish, and when.

Please test only against your own installation. Do not access, change or delete data
that is not yours, and do not run load tests against servers you do not operate.

## What to expect

Autoxide is maintained by a small team, so these are targets, not guarantees:

| Step                                       | Target                                 |
| ------------------------------------------ | -------------------------------------- |
| We confirm we received the report          | within 7 days                          |
| We tell you whether we accept it, and why  | within 14 days                         |
| A fixed release for an accepted report     | as soon as possible, by severity       |
| Public advisory                            | with the fixed release, at most 90 days after the report |

We credit reporters in the advisory and the changelog unless you ask us not to. There is
no bug bounty.

## What is not a vulnerability

These are known behaviours or outside the project, not security bugs:

- **Settings the documentation warns about**: `/metrics` reachable from the internet
  after you turned on `AUTOXIDE_HTTP_METRICS`, `AUTOXIDE_HTTP_CORS_ORIGINS=*`, rate
  limiting turned off with `0`, `AUTOXIDE_HTTP_CLIENT_IP_HEADER` set without a proxy in
  front, `AUTOXIDE_SECRET_KEY` or `AUTOXIDE_AUTH_TOKEN_SECRET` left unset in production
  (the server logs a warning for each).
- **Volumetric denial of service**: flooding a server with traffic. Report it if a *single*
  small request makes the server use far more memory, CPU or disk than it should.
- **Reports from automated scanners** without a way to reproduce an actual impact, such as
  missing headers on JSON responses or version strings.
- **Vulnerabilities in a dependency** that Autoxide cannot reach. `cargo deny` checks
  every release against the RustSec advisory database; if you found a path that does reach
  the vulnerable code, that *is* a report.
- **Problems in the user interface** or other clients: they live in their own repositories.
  Autoxide only serves an API.
- **Anything that needs an administrator account** to do what administrators may already
  do (for example, an administrator reading user data).
