- Module `ledger`: postings lock their accounts before checking status and floor, so raising a
  floor or closing an account at the same time can no longer let a posting through below the new
  floor or into the closed account (Postgres). Account updates no longer overwrite a concurrent
  change.
