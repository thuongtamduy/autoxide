- Issuing an invoice document when the unit has no active series for the document's kind and year of issue now
  returns `409 invoices.no_active_series` (parameter `year`) instead of `404 invoices.series_not_found`; create a series
  with `POST /api/v1/invoices/series`. `invoices.series_not_found` now only means the series pinned on the document
  does not exist (D-20261127-no-active-series-code).
