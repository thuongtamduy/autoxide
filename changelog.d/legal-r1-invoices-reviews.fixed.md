- Cancelling an e-invoice already sent to the tax authority is refused with `409 invoices.einvoice_not_cancellable`
  when the seller's tax jurisdiction forbids it (Vietnam since 2025-06-01, Decree 70/2025/ND-CP — tax pack rule
  `einvoice.cancel_after_submission`, pack `vn` version 3), pointing to an adjustment or a replacement. Before, every
  cancellation through SePay failed with `not_supported`. An e-invoice the tax authority rejected is cancelled locally.
- Order documents (`GET …/orders/me/orders/{id}/documents` and the staff and guest variants) have
  `differs_from_order`: `true` when the tax of an invoice line, or the invoice total, differs from what the order
  settled by more than one minor unit per line (e.g. when the VAT rate changed between ordering and invoicing), and
  always for an order mixing tax-inclusive and tax-exclusive prices, which one invoice cannot carry exactly.
- The invoice of an order priced without tax now carries the shipping fee at its pre-tax amount: the order treats the
  shipping fee as tax-inclusive, and before the invoice added the tax a second time. The invoice total can still
  differ from the order total by one minor unit from rounding the shipping tax again.
- Hiding a product review in `orders` now asks the legal rule `moderation.hide_reasons` (module `compliance`) through
  `ModerationPolicy`, which moved to `autoxide_kernel::moderation` (still re-exported by `autoxide_compliance::api`);
  a reason a tenant added to its own list can no longer hide a review.
- Voiding a billing period of `plans` whose e-invoice cannot be cancelled issues a full decrease adjustment instead
  (once, reference `plans:<period>:void`). Adjustments and replacements, also when their draft is edited, are taxed
  at the date of the invoice they correct. A tenant whose jurisdiction has no `moderation.hide_reasons` rule is not
  limited when hiding reviews.
- Cancelling an already cancelled document again now finishes the ledger reversal if it failed the first time
  (exactly one reversal, also under concurrent calls). Voiding a billing period whose invoice was already
  replaced or corrected by another document logs a warning instead of retrying.
- Refusing to hide a review for a reason the legal rule does not allow names the `orders` reason sent and the allowed
  `orders` reasons.
