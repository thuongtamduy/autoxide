- Module `organizations`: public store information for websites (step 55).
  `GET /api/v1/organizations/public/storefront` returns the brand (name, logo, site icon, phone,
  email, social profiles) inherited down the organization tree, the unit's public location profile,
  its time zone and whether it is open now. The brand is managed with the new unit settings
  `organizations.brand_name`, `organizations.brand_logo`, `organizations.brand_icon`,
  `organizations.contact_phone`, `organizations.contact_email` and `organizations.social_links`.
