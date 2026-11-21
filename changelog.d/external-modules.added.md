- Modules can live in a repository of their own, without forking Autoxide: the `autoxide` crate is
  now also a library with one entry point, `autoxide::Autoxide::new().module(your_crate::module).run()`,
  which runs the standard command line and server with the official modules plus yours. Added modules
  get migrations, permissions, events, jobs, OpenAPI routes, MCP tools, translations and settings like
  built-in ones. Guide: `docs/guides/writing-a-module.md`; working example (its own Cargo workspace,
  tested on SQLite and PostgreSQL by CI): `examples/external-module`.
- `cargo xtask check-module <dir>` runs the module rules of `cargo xtask check` on a crate outside
  the repository.
- The module contract now states what is stable for modules kept elsewhere (`autoxide-kernel`,
  `autoxide-testkit`, `autoxide::Autoxide`, the `api` of official modules) and how it may change
  before and after 1.0 (`docs/MODULE_GUIDE.md`, "Stability of the module contract").
