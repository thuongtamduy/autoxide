- Module `orders`: checkout and point-of-sale orders write all order lines and their taxes with
  one statement per table instead of one per line, so placing a large order holds the write lock
  for less time. Responses do not change.
