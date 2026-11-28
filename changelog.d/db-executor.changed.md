- Kernel (Rust): functions that read or write alongside a module — `Outbox::record`, `SyncLog::changed` /
  `deleted`, `Principal::check` / `require_at` / `require_write_at` / `reaches`, and
  `OneTimeTokens` — now take `&impl autoxide_kernel::db::Executor` instead of a SeaORM `ConnectionTrait`.
  Every handle of `db` (`tenant.db().read()`, `begin_write()`) and every connection of the old path is an
  `Executor`, so existing code compiles unchanged.
