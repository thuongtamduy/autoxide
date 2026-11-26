- Module `tax`: for tax-inclusive prices, rounding no longer produces a negative tax, or a non-zero amount for a
  0% rate or a per-unit tax whose quantity is 0, in the document totals or on a line. For documents without
  per-unit taxes, the rounding remainder still goes to the last taxed stage and the last taxed column still takes
  each line's remainder; only when that would go below zero (or land on a 0% stage) is it moved to an earlier stage,
  or swapped between lines, keeping every line total and every column total exact. Such documents that were already
  free of those amounts compute exactly as before; those that change, by one or a few minor units per stage, are
  chains with a 0% stage after a taxed one (`[k%, 0%]`), chains of two or more taxed stages where the last stage
  rounded to less than the remainder to absorb (for example `[65%, 10%, 10%]` on a total of 5), and multi-line
  groups with two or more taxed stages where a line used to receive a negative tax. Documents with per-unit taxes
  follow the per-unit entry. A stored document in one of those cases that is recomputed can differ from its stored
  figures.
