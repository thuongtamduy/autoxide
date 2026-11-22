- Module `einvoice_sepay` (experimental; written from the provider's public documentation, not yet verified against
  the live service): sends issued invoices to the tax authority through SePay eInvoice (e-invoice provider `sepay`).
  Each tenant connects its own account (`einvoice_sepay.connection.api.*` settings). VAT and sales invoices in VND,
  replacements and adjustments, status until the tax authority answers, PDF / XML download. New settings
  `AUTOXIDE_EINVOICE_SEPAY_API_URL` and `AUTOXIDE_EINVOICE_SEPAY_SANDBOX_API_URL` (per-tenant `environment`). Guide: `docs/guides/sepay-einvoice.md`.
