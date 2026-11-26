- Tax and invoice dates follow the seller's tax jurisdiction, not UTC: the tax point of an order and the issue
  date of an invoice are the calendar day in Vietnam time (`Asia/Ho_Chi_Minh`) for a seller in jurisdiction
  `VN`. Before, an order or invoice made between 00:00 and 07:00 Vietnam time carried the previous day — on
  1 January 2027 that would still apply the 8% VAT that ends on 31 December 2026. New `TaxEngine::tax_date`;
  the bundled tax pack `vn` version 2 adds the rule `tax_date.time_zone` and cites Resolution 204/2025/QH15
  and Decree 174/2025/ND-CP for the 8% VAT.
- Same tax date everywhere a date is inferred: ledger entries without an explicit date (order revenue, output VAT,
  marketplace withholding) are dated in the tax jurisdiction's time zone; a draft invoice computed on another
  day is re-taxed when issued; adjustments and replacements tax at the date of the invoice they correct; the MCP
  tool `tax.rates` defaults to today in the jurisdiction's time zone. `autoxide doctor` warns about an imported
  tax jurisdiction without a time zone rule. New error `invoices.draft_changed` (409) when a draft is edited
  while it is being issued.
