- Modules `idempotency`, `esms` and `einvoice_sepay` use only the kernel's layers (module manifest, `db` records, kernel
  `async_trait`) and no longer depend on `sea-orm`, `jiff`, `async-trait`, `axum` or `utoipa`. Behaviour, error codes
  and the API are unchanged.
