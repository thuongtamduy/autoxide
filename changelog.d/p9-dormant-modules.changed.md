- Turned-off modules (`AUTOXIDE_MODULES`) keep their duties toward personal data ("dormant mode"): a
  person's data export includes their part and deleting an account deletes the person's data in them;
  deleting a tenant for good deletes their rows, a tenant data export includes them, and moving,
  splitting or merging a tenant takes their tables along (their own migrations are applied first, only
  where they already have tables). Moving / splitting / merging no longer answers
  `409 tenant_move_modules_off`. Modules: new `Module::dormant` hook (default: nothing) with
  `DormantContext::{personal_data, on_user_deleted}`; see `docs/MODULES.md`.
