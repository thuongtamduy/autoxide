# Security policy

**English** · [Tiếng Việt](SECURITY.vi.md)

## Supported versions

Autoxide is in its `0.x` series. Security fixes go into the **latest minor release** only
(for example `0.11.x` while `0.11` is the newest); older minor releases are not patched.
Update to the latest release before reporting, and say in your report which version you
tested. The `CHANGELOG.md` lists every release.

| Version              | Security fixes |
| -------------------- | -------------- |
| Latest `0.x` release | Yes            |
| Older releases       | No             |

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
