- Kernel: `autoxide_kernel::pagination::text_key_cursor` and `PageRequest::text_key_after` — cursor for a list
  ordered by an expression that is not one column (such as a `SUM` of a `GROUP BY`), so it pages with a keyset in
  the database. The `analytics` ranking lists use it.
