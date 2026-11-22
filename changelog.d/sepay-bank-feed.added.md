- Module `sepay` (experimental; written from the provider's public documentation, not yet verified against the live
  service): confirms VietQR bank transfers from SePay's bank transaction notifications. Each tenant connects its own
  SePay account (`sepay.connection.bank.*` settings); signed webhooks (HMAC-SHA256 or API key) at
  `POST /api/v1/ledger/bank-feeds/sepay/{hook}` and the SePay transaction API for missed notifications. New setting
  `AUTOXIDE_SEPAY_API_URL`. Guide: `docs/guides/sepay.md`.
