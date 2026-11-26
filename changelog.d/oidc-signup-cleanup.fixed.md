- Module `auth`: when creating an account through a sign-in provider fails half-way (for example the plan's staff
  limit is reached), the account just created is removed instead of being left without a role or a link. Saving a
  sign-in provider whose default role you cannot give now says how to go on (ask someone who can give the role, or
  remove the default role), and the sign-in callback documents `auth.oidc_default_role_unavailable`.
