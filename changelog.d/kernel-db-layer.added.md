- Kernel: a database layer for modules, `autoxide_kernel::db`, so a module no longer depends on SeaORM. A table row is a
  `#[derive(Record)]` struct with typed columns (`Note::TITLE.eq(42)` does not compile), and `tenant.db()` filters every
  read, write, join and subquery by the tenant. An insert of another tenant's row is refused, and `ON CONFLICT DO UPDATE`
  never overwrites another tenant's row. The derive refuses, at compile time, a table that does not belong to the module.
  At startup the kernel checks each record a module declares (`Module::records`) against the live schema. The sample
  module `notes` uses it.
