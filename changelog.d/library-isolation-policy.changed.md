- Compatibility policy: a new major version of a library under the core (`sea-orm`, `axum`, `utoipa`,
  `validator`, `jiff`…) no longer makes a major release of Autoxide. Modules reach those libraries
  through the kernel's layers (`db`, `http`, `migration`, `time`), and only the vocabulary libraries
  (`serde`, `serde_json`, `uuid`, `tokio`, `tracing`, `futures-core`, `bytes`) are part of the module
  contract (`docs/COMPATIBILITY.md`, `docs/MODULE_GUIDE.md`).
