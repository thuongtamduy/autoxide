- Module `auth`: `POST /api/v1/auth/scim/group-mappings` gives the role to every current member of the group even
  when some of them fail (for example the plan's staff limit is reached), and answers `201` with `granted_user_ids`
  (members who hold the role now) and `failed` (`user_id`, `code`, `message`) instead of an error after the rule was
  already saved. The plan's staff count now follows who actually holds a role after a SCIM sync.
- Module `auth`: a SCIM token can no longer change any attribute (`userName`, `externalId`, the name) of an account
  with permissions its creator lacks — before, only the email and `active` were checked. `userName` and
  `externalId` are what the identity provider matches people by.
- Module `auth` (security): when SCIM changes the email of a linked account, the account is signed out everywhere and
  its password, passkeys, two-step sign-in, API keys, pending sign-in links and external sign-in links are removed in
  the same step. Someone who held the account under the old email can no longer keep it after the identity provider
  hands it to the real person. After a group write, members still missing a role the group's rules give are synced
  again (at most 100 each time); reads that decide roles right after a write no longer use a read replica. SCIM
  tokens the account created are revoked too, and a SCIM token stops giving or taking away anything once its creator
  no longer holds `auth.scim.manage`.
- **Action needed:** a SCIM email change revokes that account's API keys. Machine-to-machine integrations that use an
  API key of a person's account stop working when the identity provider changes that person's email (for example
  when the company moves to a new email domain). Run integrations under a separate service account whose email the
  identity provider does not manage.
