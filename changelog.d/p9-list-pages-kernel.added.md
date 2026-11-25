- Kernel: `autoxide_kernel::pagination::NaturalOrderPageQuery` (lists in a natural order: `order` defaults to
  `asc`), `SortChoicePageQuery` with `into_request` (lists with a `sort` choice: the default of
  `order` follows the chosen sort) and `page_of` (one page of a list that is not in the database).
