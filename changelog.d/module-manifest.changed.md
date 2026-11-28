- Rust API (breaking before 1.0): `module::Stability`, `module::SelectionError`, `module::InactiveModule` and
  `module::DormantModule` are `#[non_exhaustive]`; add a `_ =>` arm when matching the enums. `Module::name` now has
  a default (the manifest's name).
