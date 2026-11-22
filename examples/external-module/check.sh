#!/usr/bin/env bash
# Checks the example the way the gate of Autoxide checks the core: format, lints, then every test
# on SQLite and — when a PostgreSQL server is given — on PostgreSQL.
#
#   ./check.sh                                         SQLite only
#   POSTGRES_URL=postgres://user:password@host:5432/postgres ./check.sh
#                                                      SQLite, then PostgreSQL (the user must be
#                                                      allowed to CREATE DATABASE)
#
# In a repository of your own, keep a script like this one and run it in CI.
set -euo pipefail
cd "$(dirname "$0")"

# Inside the Autoxide repository only: resolve the same library versions as the core, so that this
# example is tested against exactly what the core is tested with. (In your own repository, commit
# your `Cargo.lock`; to start from the versions a release was tested with, copy the `Cargo.lock`
# of its tag and let cargo trim it.)
if [ -f ../../Cargo.lock ]; then
  cp ../../Cargo.lock Cargo.lock
fi

# On CI every program is built once and run once: building the libraries unoptimized is faster than
# the time their tests save. On your machine the profile of Cargo.toml (opt-level 1) stays.
PROFILE=()
if [ -n "${GITHUB_ACTIONS:-}" ]; then
  PROFILE=(--config 'profile.dev.package."*".opt-level=0')
fi

echo "== format"
cargo fmt --check -p acme-bookmarks -p acme-server
echo "== lints"
cargo clippy --workspace --all-targets ${PROFILE[@]+"${PROFILE[@]}"} -- -D warnings
echo "== tests on SQLite"
env -u TEST_DATABASE_URL cargo test --workspace ${PROFILE[@]+"${PROFILE[@]}"}
if [ -n "${POSTGRES_URL:-}" ]; then
  echo "== tests on PostgreSQL"
  # Every test creates a database of its own: keep the number of tests running at once below the
  # connection limit of the server.
  TEST_DATABASE_URL="$POSTGRES_URL" cargo test --workspace ${PROFILE[@]+"${PROFILE[@]}"} -- --test-threads=4
else
  echo "== tests on PostgreSQL: skipped (set POSTGRES_URL to run them)"
fi
echo "example: all checks passed"
