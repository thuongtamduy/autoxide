- Kernel (Rust): more functions take `&impl autoxide_kernel::db::Executor` instead of a SeaORM `ConnectionTrait`,
  so a `db` handle can be passed directly: `JobQueue::enqueue` / `enqueue_database`, `job::runs::track` /
  `progress` / `set_result`, every `OrgTree` and `DirectUploads` function, `SearchIndex::enqueue`,
  `Distribution::publish`, `custom_fields::write_index`, `numbering::next` / `current`, `Checked::claim` and `SpamGuard::purge_nonces` (antispam).
  Existing callers compile unchanged.
