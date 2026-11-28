- SCIM: a group's mapping rules give their roles only to members whose SCIM token (its creator, as
  of now) could give them; the others wait for approval. `POST /api/v1/auth/scim/group-mappings`
  lists them in `waiting_user_ids`, and `POST
  /api/v1/auth/scim/groups/{id}/members/{user_id}/approve` approves one. A low-rights token can no
  longer add people to a group without rules and have an administrator's rule raise them. Members
  added before this release wait too for rules added from now on (roles already given stay).
- SCIM: when a sync raises an account above the person whose SCIM token created it, every old way
  into the account (password, sessions, API keys, passkeys, two-factor setup, sign-in links,
  external sign-in links) is revoked first, as when SCIM changes its email. This also covers identity
  providers that never send emails.
