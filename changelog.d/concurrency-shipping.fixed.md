- Module `shipping`: two COD statements that list the same shipment (different references, or
  one statement repeating a line) settle it once — the carrier COD was credited twice on Postgres
  (and for a repeated line on SQLite too). Two people resolving the same COD discrepancy at once:
  the second gets `409` instead of overwriting the first resolution.
