- Faster public shop reads, same responses: the product list and its facets look up seller
  names once per page instead of once per seller (new `OrgDirectory::units`; its default
  implementation keeps older implementations working); the product page no longer reads image
  `alt` overrides and option counts twice; the deprecated `GET /content/public/menus/{key}` reads
  all linked terms in one statement.
