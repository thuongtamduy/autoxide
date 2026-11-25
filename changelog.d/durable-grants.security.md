- Module `auth`: disabling an account (over REST or with SCIM `active: false`), and every path that locks an account
  out, now also revokes the SCIM tokens the account created and stops sign-in providers it configured from linking
  external identities to existing accounts automatically (existing links keep working). A sign-in provider no longer
  links automatically once the person who configured it lost `auth.oidc.manage`. Before, these kept the power of the
  account's later permissions, and enabling the account again brought SCIM tokens back.
- **Action needed:** create the SCIM token from a separate staff account whose email the identity provider does not
  manage. Changing that person's email or disabling them revokes the token the provider is using, and syncing stops
  with `401` (for example when the company moves to a new email domain). After disabling the person who configured a
  sign-in provider, save the provider again from another administrator's account to restore automatic linking.
