- Kernel: an HTTP layer for modules, `autoxide_kernel::http`, so a module no longer depends on axum, utoipa or
  validator. A handler takes `#[endpoint(…)]` (the grammar of `utoipa::path`) and must say who may call it:
  `permission = "…"`, `signed_in` or `public`, plus `approval = "agent"` for calls an AI agent must have
  approved. A missing declaration does not compile, and `x-permissions`, `x-public` and `x-approval` are generated, never
  written by hand. The standard 401 and 403
  responses, `operation_id` and the `ETag` header (`etag`) are filled in. `routes![…]` builds the module's routes for the
  new `Module::endpoints`. Types use `#[derive(Schema)]`, `#[derive(Params)]` and `#[derive(Validate)]`. The validation
  errors are the same as before. Handlers take `Path`, `Query` and `State` from the kernel, and answer with `NoContent` or
  `Download`. `autoxide_kernel::async_trait` replaces the `async-trait` crate. The sample module `notes` uses all of it,
  and its OpenAPI contract is unchanged.
