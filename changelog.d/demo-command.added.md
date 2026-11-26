- `autoxide demo`: try Autoxide in one command, with no setup. It creates a folder, a SQLite database, random
  secrets and an administrator whose password is printed on screen, loads the reference data and serves the API
  with its reference page at `/api/docs`. Also `docker run --rm -it -p 127.0.0.1:8080:8080
  ghcr.io/thuongtamduy/autoxide:latest demo`. See `docs/GETTING-STARTED.md`.
- Release pages now carry binaries for macOS (arm64, x86_64) and Windows (x86_64) next to the Linux ones; they are
  not signed with an Apple or Microsoft certificate yet (`docs/GETTING-STARTED.md` says how to open them).
- `cargo xtask doctor` checks that a developer's machine has the tools the repository needs, with one install command
  for each missing one; every violation of `cargo xtask check` / `check-module` now prints a `how to fix:` line.
- A `.devcontainer/` for Codespaces and VS Code, an English `AGENTS.md` for people and agents outside the project,
  and rules for Cursor (`.cursor/rules`) and Copilot (`.github/copilot-instructions.md`).
- `autoxide demo` always uses its own SQLite file and ignores `DATABASE_URL` from the environment and `.env`, so it
  cannot touch a real database; `--database-url` points it at another database on purpose.
