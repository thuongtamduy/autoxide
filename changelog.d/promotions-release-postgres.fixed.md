- Module `promotions`: releasing the promotion usage held for an order by hand (`POST
  /api/v1/promotions/redemptions/{id}/release`) answered 500 on PostgreSQL, because the write permission was checked
  on a read-only connection.
