- Module `auth`: granting roles (memberships, invitations, SCIM group mappings, OIDC default roles)
  now compares the scope of each permission, not only its name; an invitation is checked again
  against the inviter's current permissions when it is accepted.
- Module `auth`: SCIM only links, deactivates or changes the email of accounts whose permissions the
  token's creator also has; an email changed through SCIM on a linked pre-existing account is no
  longer marked as verified.
