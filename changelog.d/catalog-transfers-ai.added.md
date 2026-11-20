- Module `catalog`: CSV import (`POST /api/v1/catalog/imports`) and export (`POST /api/v1/catalog/exports`)
  run as background jobs, with errors by row, re-import by SKU without duplicates and protection
  against CSV formula injection; MCP tools `catalog.start_import` and `catalog.import_status`.
- Module `catalog`: AI drafts of product descriptions and SEO texts (`POST /api/v1/catalog/products/{id}/suggestions`,
  then `apply` or `discard`); applying marks the product `ai_assisted`. MCP tool `catalog.suggest_copy`.
- Module `catalog`: event `catalog.display_price_changed` when a product's public price changes.
