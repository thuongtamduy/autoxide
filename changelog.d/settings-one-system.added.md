- Kernel: one settings system for both scopes, `tenancy::SettingSystem` (`context.require::<SettingSystem>()`).
  Per-node settings inherited down the organization tree are read through it with a per-tenant cache
  (`effective`, `effective_all`; a cache hit runs no query, a write on this server clears it at once), and modules
  register save-time cross-checks over keys of either scope (`SettingRule`, `SettingsView`): saving a value that
  breaks a rule answers 422 `validation_failed` naming what to fix. A key may live in one scope only. The
  `organizations` module plugs its values in (`OrgSettingStore`), so every `OrgDirectory::setting` read is cached.
