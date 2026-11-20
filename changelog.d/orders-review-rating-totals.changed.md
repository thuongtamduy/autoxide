- Module `orders`: events `orders.review_published` and `orders.review_moderated` now carry
  `product_rating` (`count`, `sum`: the product's current totals), which `catalog` uses for ratings.
