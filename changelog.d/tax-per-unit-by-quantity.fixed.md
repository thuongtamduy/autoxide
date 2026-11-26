- Module `tax`: a per-unit tax is now split across the lines of a document by each line's quantity, not by its
  amount, so a line with quantity 0 no longer carries part of it — for tax-exclusive and tax-inclusive prices.
  When a chain includes a per-unit tax, the net and the other taxes of each line follow that line's own figures
  too (for example, a percent tax after a per-unit tax is split by each line's amount plus its per-unit tax). For
  tax-inclusive prices the rounding remainder now goes to a percent stage before a per-unit stage, so the per-unit
  total stays at the unit amount times the quantity; a line whose amount is lower than the per-unit taxes it
  includes is rejected (`range` on `lines[i].amount`), and a document is rejected only when its per-unit taxes do
  not fit in the lines that have a quantity. Documents without per-unit taxes compute exactly as before; documents
  with per-unit taxes on several lines, or whose rounding remainder fell on a per-unit stage, can change.
- Module `tax`: a tax pack is rejected when a per-unit amount is negative, a rate is above 1000%, or a withholding
  rate is above 100%.
