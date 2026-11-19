- `catalog` (step 41): search suggestions as the shopper types at
  `GET /api/v1/catalog/public/search/suggest`, gathered from every module through the new kernel trait
  `autoxide_kernel::suggest::SuggestSource` (exact Vietnamese accents first); per-tenant search synonyms
  (`GET` / `PUT /api/v1/catalog/synonyms`); filter counts per attribute value and per seller at
  `GET /api/v1/catalog/public/products/facets`.
