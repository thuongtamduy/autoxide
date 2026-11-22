- Guide "Opening a shop" (`docs/guides/shop.md`, Vietnamese `shop.vi.md`): from the quickstart to a first
  order paid with MoMo, delivered by GHN, with SMS through eSMS and an e-invoice through SePay eInvoice;
  the same flow runs as an end-to-end test on SQLite and PostgreSQL.
- Module `ledger`: the tenant setting `ledger.payments.default_provider` picks the payment partner when
  a payment does not name one and the server has several.
