- On a marketplace (`kernel.business_model = marketplace`) with module `compliance`, a product of a seller (any branch
  other than the tenant's root) can only go `active` once the seller's identity is verified; otherwise
  `POST /api/v1/catalog/products/{id}/status` and the MCP tool `catalog.set_product_status` answer
  `compliance.kyc_required` (403). Whether verification is required follows the dated rule `kyc.required_activities`
  (E-commerce Law 122/2025/QH15, Decree 248/2026/ND-CP Article 18). Goods already on sale are not re-checked.
