- `catalog` (step 41): `autoxide_catalog::api::CatalogExtensions::set_listing_gate` lets a module
  (the marketplace) refuse to put a product on sale; the status change then answers 422
  `catalog.listing_denied` with the module's reason.
