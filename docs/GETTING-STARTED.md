# Getting started

Three ways to try Autoxide, from least to most work. All three give you the same server: the REST API,
the OpenAPI document, the MCP endpoint for AI agents, and an administrator account.

## 1. The demo (about two minutes)

`autoxide demo` is a throw-away server for looking around. It needs no settings. On first start it
creates the folder `autoxide-demo` (change it with `--dir`) with a SQLite database, random secrets, an
administrator `admin@example.com` and the reference data (addresses, taxes, promotion limits). It prints
the password on screen. Run it again and it continues with the same data. Stop it with Ctrl+C.

With Docker:

```bash
docker run --rm -it -p 127.0.0.1:8080:8080 ghcr.io/thuongtamduy/autoxide:latest demo
```

The data is gone when the container stops. To keep it, add a volume and put the demo folder in it:

```bash
docker run --rm -it -p 127.0.0.1:8080:8080 -v autoxide-demo:/var/lib/autoxide/data \
  ghcr.io/thuongtamduy/autoxide:latest demo --dir data
```

With a downloaded binary: take the file for your system from the
[release page](https://github.com/thuongtamduy/autoxide/releases/latest), unpack it and run
`./autoxide demo` (`autoxide.exe demo` on Windows). The release files have a checksum and a build
attestation: [`VERIFYING.md`](VERIFYING.md). The macOS and Windows files are not signed with an Apple
or Microsoft certificate, so the system may warn the first time:

- macOS: `xattr -d com.apple.quarantine ./autoxide`, then run it again.
- Windows: SmartScreen, "More info", "Run anyway".

Then open `http://127.0.0.1:8080/api/docs` for the API reference and sign in with the printed
administrator account (`POST /api/v1/auth/login`).

The demo always uses its own SQLite file in the demo folder: it ignores `DATABASE_URL` from your environment and `.env`, so it cannot touch a real database. `--database-url <url>` points it at another database on purpose.

Not for production: the secrets sit in a plain file in the demo folder, and the API reference page
loads its viewer script from a public CDN.

## 2. The quickstart (a real single-server install)

Docker Compose, a volume for the data, secrets of your own: see the Quickstart in the
[README](../README.md#quickstart). For PostgreSQL, several servers and HTTPS: [`DEPLOY.md`](DEPLOY.md).

## 3. Working on Autoxide itself

Open the repository in a Codespace or a dev container (`.devcontainer/` is ready), or on your own machine
check what is missing:

```bash
cargo xtask doctor      # read-only: lists the tools you need, with one install command each
./gate.sh --quick       # what "done" means before a commit
```

When `cargo xtask check` reports a violation it prints a `how to fix:` line under it. Rules for
contributors and AI agents: [`AGENTS.md`](../AGENTS.md) and [`CONTRIBUTING.md`](../CONTRIBUTING.md).
