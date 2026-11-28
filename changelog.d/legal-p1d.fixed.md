- Module `orders`: order, POS sale, return and warranty claim numbers take the year in the seller's time zone (the
  tax date), not UTC — a sale at 00:30 on 1 January in Vietnam is numbered in the new year.
- Module `orders`: the invoice of an order follows the tax data of the seller — a household business gets a sales
  invoice instead of a VAT invoice (unless the seller set `orders.invoice_kind` itself), and none is issued
  automatically when the law does not require one — an invoice the buyer asks for is still issued.
- Module `compliance`: a `kyc.required_activities` rule body that is not JSON makes the identity gate fail instead of
  letting everyone through.
- Module `catalog`: a failure to read the branch chain when checking a seller's identity is reported instead of being
  ignored.
