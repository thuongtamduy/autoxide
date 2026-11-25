- Module `auth`: a SCIM token, and the SCIM group mappings, can no longer give, keep or take away more than the person
  responsible holds (2026-11 security review, P-7). Before, someone holding only `auth.scim.manage` could use a SCIM
  token they created to give accounts a role that a group mapping made by an administrator grants (including
  `admin`), delete accounts with more permissions than their own, and remove an administrator's group mappings.
  Now adding or removing group members, deleting a group, deleting a SCIM user and deleting a group mapping all
  check the token creator's (or the caller's) current permissions and answer `403` (SCIM error body; REST:
  `auth.role_beyond_you`) without changing anything. A token's power follows its creator at the time of use: a creator
  who is demoted, disabled or deleted takes the token's power down with them. Tenants using SCIM with
  administrator-level mappings should create the SCIM token from an account that holds those roles.
