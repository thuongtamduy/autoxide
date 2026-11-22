# acme-bookmarks

An [Autoxide](https://github.com/thuongtamduy/autoxide) module in a repository of its own, and the
server `acme` that runs it next to the official modules. [Tiếng Việt](README.vi.md)

| Path | What it is |
| --- | --- |
| `bookmarks/` | the module: tables, service, REST routes with OpenAPI, MCP tools, jobs, settings, tests |
| `server/` | the binary: the official modules plus this one |
| `check.sh` | format, lints, and every test on SQLite and PostgreSQL |
| `.github/workflows/ci.yml` | CI that runs `check.sh` on both databases |
| `AGENTS.md` | rules for AI agents working here (`CLAUDE.md` links to it) |

## Run it

```bash
./check.sh                                  # SQLite only
POSTGRES_URL=postgres://postgres:postgres@127.0.0.1:5432/postgres ./check.sh
cargo run -p acme-server -- migrate up
cargo run -p acme-server -- serve
```

The API document is at `/api/v1/openapi.json` and the MCP endpoint at `/mcp`.

## Next

1. Rewrite the table, the service, the descriptions and `bookmarks/locales/vi.json` for your feature,
   and rename `acme` to a name of your own.
2. Check the architecture rules from a checkout of Autoxide:
   `cargo xtask check-module <path>/bookmarks`.
3. To upgrade Autoxide, change the `tag` of the three `autoxide*` lines in `Cargo.toml`.

Every part is explained in the guide
[Writing a module in your own repository](https://github.com/thuongtamduy/autoxide/blob/main/docs/guides/writing-a-module.md).
