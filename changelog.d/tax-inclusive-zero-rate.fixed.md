- Module `tax`: for tax-inclusive prices, the rounding remainder no longer lands on a stage whose tax is
  zero (a 0% rate, or a per-unit tax that rounds to zero), which used to produce a tax of -1 minor unit.
  For chains such as `[k%, 0%]` the amount of stage k and the totals per (kind, rate) change by one minor
  unit compared with before, so a stored document that is recomputed can differ from its stored figures.
