- Module `catalog`: product collections (step 55). `/api/v1/catalog/collections` (new permission
  `catalog.collections.manage`) keeps manual collections (chosen products in order) and rule
  collections (categories, attributes, price range, new products, on promotion; at most 10
  conditions), sorted by chosen order, newest, price or best sellers, with texts and SEO per
  language. `GET /api/v1/catalog/public/collections/{slug}` shows a collection page with the same
  product cards as the product list. Collections are in the object directory
  (`catalog.collection`) for banners and menus. MCP tools `catalog.list_collections` and
  `catalog.save_collection` (drafts only).
