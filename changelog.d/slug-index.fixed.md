- Creating a content entry or a product whose slug is generated from its title no longer scans every row with the
  same language on PostgreSQL: the free slug is looked up in groups of candidates (`slug`, `slug-2` … `slug-20`,
  then the next group), which uses the unique index on SQLite and PostgreSQL alike. Entries or products created at
  the same time with the same title each get their own slug instead of one of them failing with `slug_taken`.
