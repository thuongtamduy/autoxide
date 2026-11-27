- `autoxide config` and the start-up log of `autoxide serve` name an `AUTOXIDE_…` variable that is not a setting (a
  misspelling such as `AUTOXIDE_PORT` used to be ignored in silence) and suggest the closest real name.
- `docs/DEPLOY.md`: "B+", one binary with PostgreSQL from scratch, and "3b", trying the binary by hand (`migrate up`
  before `serve`, `AUTOXIDE_HTTP_ADDRESS`, a first request with data); the README points to them for people without Docker.
