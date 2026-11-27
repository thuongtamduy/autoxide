- Kernel: an HTTP layer for modules, `autoxide_kernel::http`, so a module no longer depends on axum, utoipa or
  validator. A handler takes `#[endpoint(…)]` (the grammar of `utoipa::path`) and must say who may call it:
  `permission = "…"`, `signed_in` or `public`. A missing declaration does not compile. The standard 401 and 403
  responses, `operation_id` and the `ETag` header (`etag`) are filled in. `routes![…]` builds the module's routes for the
  new `Module::endpoints`. Types use `#[derive(Schema)]`, `#[derive(Params)]` and `#[derive(Validate)]`. The validation
  errors are the same as before. Handlers take `Path`, `Query` and `State` from the kernel, and answer with `NoContent` or
  `Download`. `autoxide_kernel::async_trait` replaces the `async-trait` crate. The sample module `notes` uses all of it,
  and its OpenAPI contract is unchanged.
