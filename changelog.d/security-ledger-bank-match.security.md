- Module `ledger`: a bank credit only settles a VietQR payment when it arrived in the bank account
  that payment expects; otherwise it goes to the reconciliation queue. Two concurrent `match`
  resolutions of one queue item can no longer settle two payments with one credit.
- Module `ledger`: a payment stores the amount actually collected (`captured_minor`, new column in
  migration `m0012_add_captured_amount`), and refunds are limited to it, so a short payment accepted
  as received cannot be refunded up to the expected amount. A refund notification only completes a
  refund of a payment made through the same provider.
