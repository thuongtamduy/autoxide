- **Request bodies refuse unknown fields everywhere.** Most endpoints already answered a field they do
  not know (often a misspelt one) with `422 invalid_body`; the remaining ones — sign-in, sessions,
  password and email changes, users and roles in `auth`, tags, notes, views, texts, approvals, bulk
  actions and exports in the kernel, address validation, a few `catalog`, `forms` and `promotions`
  bodies — now do the same instead of ignoring it, and their schemas say `additionalProperties:
  false`. A client that sent extra fields to these endpoints must stop sending them. SCIM 2.0,
  partner callbacks, the cart line (`PUT /api/v1/orders/carts/{id}/lines/{variant_id}`) and bodies
  built from several parts are unchanged.
