- Module `auth`: SCIM `Groups` (filters, PATCH of `members` as Entra ID and Okta send it) and mapping
  rules from a group to a role at an organization unit (`/api/v1/auth/scim/group-mappings`,
  `GET /api/v1/auth/scim/groups`). Joining or leaving a group grants or removes the role at once;
  roles given by hand are never removed by SCIM. Groups without a rule appear in the work inbox
  (`auth.scim_unmapped_groups`).
