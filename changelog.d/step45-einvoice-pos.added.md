- Module `orders`: electronic invoices for orders through `invoices`, at the time the shop's policy
  says (`orders.invoice_timing`, by default on delivery) or on request with company details; an
  adjustment invoice after a return; buyers list and print their documents (also with an
  `Order-Token`); order confirmation and status messages through `notifications`.
- Module `orders`: point of sale for chains — cash register shifts (one open shift per register),
  sales at the counter that take stock from the store only, cash with change, card terminal and
  VietQR, the same `reference` returning the same sale, a cash register invoice; closing a shift
  books the cash difference with a listed reason, and a difference above the threshold waits for
  another person's approval.
- Module `invoices`: `NewDocument::booked_by_source` (the source module already booked the
  revenue, issuing books nothing) and `Invoices::adjust` for adjustment invoices from other modules.
