- Module `tax`: with prices that include tax, a tax chain ending in an exempt rate no longer puts the rounding
  remainder (e.g. `-1 VND`) on the exempt rate; it goes to the last taxed rate, so exempt lines always show zero tax.
- Module `tax`: a quantity too large to compute (up to 28 digits are accepted) is a `validation_failed` error instead
  of a 500, a negative quantity is rejected, and a tax-inclusive price lower than the per-unit taxes it includes is
  rejected instead of producing a negative net price.
- Module `promotions`: carts whose line totals reach the `i64` limit no longer crash the discount calculation.
