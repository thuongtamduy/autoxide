- `autoxide modules` lists every module of the program — official ones and those a binary of your
  own added — with its tier, whether it is on, what it depends on and, for modules that are off,
  the tables that still hold data. `--json` prints the same for programs and agents.
- For module authors: `autoxide_kernel::migration::helper_tables` creates a new module's event and
  job tables with one call; `autoxide_kernel::time::SignedDuration` (no need to depend on `jiff`);
  `cargo xtask new-module --external <dir> <module> <entity>` generates a workspace of your own
  (a module and its server) from `examples/external-module`.
