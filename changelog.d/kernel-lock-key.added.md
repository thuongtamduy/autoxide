- Kernel: `rows::lock_key` serialises write transactions on a name (a Postgres transaction-level
  advisory lock; nothing extra on SQLite, whose write transactions already run one at a time).
