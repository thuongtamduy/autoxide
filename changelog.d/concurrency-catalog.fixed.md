- Module `catalog`: writes to a product (status, variants, documents) lock the product row, so a
  product cannot go on sale while a required document is being removed, and two concurrent variant
  edits no longer both emit `catalog.preorder_date_changed` (Postgres).
