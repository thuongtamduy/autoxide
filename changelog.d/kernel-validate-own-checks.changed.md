- Kernel (Rust): `#[derive(Validate)]` of the kernel implements `autoxide_kernel::http::Validate` directly and runs
  the kernel's own checks (`length`, `range`, `email`, `url`, `nested`) instead of generating `validator` code. Errors
  are unchanged (same fields, codes, parameters and messages). Breaking (Rust): a type with the kernel's derive no
  longer implements `validator::Validate`, so it cannot be `#[validate(nested)]` inside a struct that still derives
  `validator::Validate` — derive the kernel's `Validate` on the outer struct too.
