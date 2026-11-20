- Module `catalog`: best sellers and ratings now come from module `orders` (completed orders over a
  rolling 90 days, review totals), so `sort=best_selling` / `sort=rating` and `min_rating` work on
  public product lists.
- Module `catalog`: `GET /api/v1/catalog/public/products/{id}/similar` (AI embeddings, or the same
  categories when embeddings are off) and `GET /api/v1/catalog/public/products/{id}/frequently-bought-together`.
