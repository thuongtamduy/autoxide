- Module `ledger`: one rule for money arriving late (after the deadline or after the partner reported the
  payment failed) on every path: a statement line or a reconciliation item can now record money for a
  `failed` payment (it was refused with 409), and a bank transfer arriving after the deadline is no longer
  counted as a clean match. Refunds answered with 409 or 429 stay `pending` and are sent again instead of
  being marked `failed`, in `ledger` and `orders` alike.
