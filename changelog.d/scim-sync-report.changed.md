- Module `auth`: `POST /api/v1/auth/scim/group-mappings` gives the role to every current member of the group even
  when some of them fail (for example the plan's staff limit is reached), and answers `201` with `granted_user_ids`
  (members who hold the role now) and `failed` (`user_id`, `code`, `message`) instead of an error after the rule was
  already saved. The plan's staff count now follows who actually holds a role after a SCIM sync.
- Module `auth`: a SCIM token can no longer change any attribute (`userName`, `externalId`, the name) of an account
  with permissions its creator lacks — before, only the email and `active` were checked. `userName` and
  `externalId` are what the identity provider matches people by.
