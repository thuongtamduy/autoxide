- Turned-off modules (`AUTOXIDE_MODULES`) keep their duties toward personal data ("dormant mode"): a
  person's data export includes their part and deleting an account deletes the person's data in them;
  deleting a tenant for good deletes their rows, a tenant data export includes them, and moving,
  splitting or merging a tenant takes their tables along (their own migrations are applied first, only
  where they already have tables). Moving / splitting / merging no longer answers
  `409 tenant_move_modules_off`. Modules: new `Module::dormant` hook (default: nothing) with
  `DormantContext::{personal_data, on_user_deleted}`; see `docs/MODULES.md`.
- Operators: `autoxide migrate up` (and migrate on start) also applies the new migrations of a turned-off
  module to a database that already has its tables, never creating them elsewhere. After an upgrade where
  a turned-off module brings a new migration, `autoxide serve` refuses to start until `migrate up` runs, as
  for any module. `autoxide migrate down <module> --steps 1000` now works on a turned-off module: once all
  its migrations are reverted it counts as not installed (nothing pending, `migrate up` does not recreate
  its tables).
