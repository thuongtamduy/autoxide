- Module `auth`: a SCIM token, and the SCIM group mappings, can no longer give, keep or take away more than the person
  responsible holds (2026-11 security review, P-7). Before, someone holding only `auth.scim.manage` could use a SCIM
  token they created to give accounts a role that a group mapping made by an administrator grants (including
  `admin`), delete accounts with more permissions than their own, and remove an administrator's group mappings.
  Now adding or removing members of a mapped group, deleting a group, deleting or changing a SCIM user with more
  permissions, and deleting a group mapping all check the token creator's (or the caller's) current permissions and
  answer `403` (SCIM error body; REST: `auth.role_beyond_you`) without changing anything.
- **Action needed:** a SCIM token's power follows its creator at the time of use. Create the token from an account
  that holds every role your group mappings give, and **rotate it when its creator leaves or loses roles** —
  otherwise removing people from groups and disabling leavers fails in the identity provider with `403`.
  `GET /api/v1/auth/scim/tokens` now shows `creator_status` (`active`, `disabled`, `deleted`, `missing_permission`)
  for each token.
- `POST /api/v1/auth/scim/group-mappings` now lists the members who received the role (`granted_user_ids`). A group
  without rules can be filled by anyone holding a SCIM token: check its members before mapping it to a powerful role.
