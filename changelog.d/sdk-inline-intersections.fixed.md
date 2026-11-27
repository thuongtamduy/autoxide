- TypeScript SDK: `ProviderRequest.realm` is now `"staff" | "customer"` (it was intersected with an object type, so no
  value could be assigned) and `CatalogPriceInput.amount` is the plain `Money` shape. The OpenAPI document no longer
  describes these two fields as both a string or object and an object, and `cargo xtask check` now rejects that shape
  (D-20261127-inline-field-docs).
