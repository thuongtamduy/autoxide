- Module `auth`: recovering an account by password reset also removes its outside sign-in links,
  and every password change revokes pending sign-in links and confirmation tokens.
- Module `auth`: a sign-in provider links an existing account by email only when whoever last
  configured the provider has all of that account's permissions (new column
  `auth_oidc_provider.configured_by`).
