- Test kit: `autoxide_testkit::http::{StatusCode, Method}` and `TestResponse::status_code()` /
  `McpClientError::status_code()`, so a module's tests no longer need `axum` or `http` in
  `[dev-dependencies]`; `autoxide_testkit::http::Client` sends requests straight to a kernel router for
  benchmarks. `TestApp::request` takes `impl Into<Method>` (the `http` crate's `Method` still works);
  `TestResponse::status()` keeps returning the `http` type until 1.0. Breaking (Rust): a method name
  parsed inline (`app.request(name.parse().unwrap(), …)`) needs `Method::from_name(name)`.
