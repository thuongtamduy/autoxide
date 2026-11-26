- Tax and invoice dates follow the seller's tax jurisdiction, not UTC: the tax point of an order and the issue
  date of an invoice are the calendar day in Vietnam time (`Asia/Ho_Chi_Minh`) for a seller in jurisdiction
  `VN`. Before, an order or invoice made between 00:00 and 07:00 Vietnam time carried the previous day — on
  1 January 2027 that would still apply the 8% VAT that ends on 31 December 2026. New `TaxEngine::tax_date`;
  the bundled tax pack `vn` version 2 adds the rule `tax_date.time_zone` and cites Resolution 204/2025/QH15
  and Decree 174/2025/ND-CP for the 8% VAT.
