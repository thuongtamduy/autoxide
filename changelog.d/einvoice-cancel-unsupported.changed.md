- Cancelling an invoice whose e-invoice provider cannot cancel a sent invoice (SePay eInvoice, e.g. when the seller has
  no tax profile) now returns `409 invoices.einvoice_cancel_unsupported` (parameter `provider`) instead of
  `422 einvoice.not_supported`; issue an adjustment or a replacement instead (D-20261127-einvoice-cancel-unsupported).
