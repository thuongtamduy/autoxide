- Kernel: new time types `autoxide_kernel::time::{Timestamp, Date}` built on `jiff` 1.0, replacing
  the unmaintained `chrono` step by step. Stored values, JSON and OpenAPI stay exactly as before;
  modules keep working while they move over.
