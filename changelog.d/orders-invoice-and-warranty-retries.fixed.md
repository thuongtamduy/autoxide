- Module `orders`: a delivered order no longer gets stuck when its invoice cannot be issued automatically (e.g. the
  shop has no document series yet): warranties, points and the other work on delivery still happen, the invoice
  stays a draft, and asking again with `POST /api/v1/orders/{id}/invoice` once the series exists issues it (it
  answered 500 before). Warranties are no longer opened twice nor block the event when a delivery event is
  delivered again.
