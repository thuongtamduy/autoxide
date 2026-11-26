- Module `tax`: a per-unit tax is now split across the lines of a document by each line's quantity, not by its
  amount, so a line with quantity 0 no longer carries part of it. When a chain includes a per-unit tax, the net
  and the other taxes of each line follow that line's own figures too. With tax-inclusive prices, a line whose
  amount is lower than the per-unit taxes it includes is now rejected (`range` on `lines[i].amount`), as a whole
  document already was. Documents without per-unit taxes compute exactly as before; documents with per-unit taxes
  on several lines with different quantity-to-amount ratios change.
- Module `tax`: a tax pack is rejected when a per-unit amount is negative, a rate is above 1000%, or a withholding
  rate is above 100%.
