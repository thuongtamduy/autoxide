- The unmaintained `chrono` and `chrono-tz` crates (RUSTSEC-2026-0543, RUSTSEC-2026-0555) are no longer
  dependencies: every time value uses `jiff` through `autoxide_kernel::time`. Stored values, JSON and the
  OpenAPI document are unchanged.
