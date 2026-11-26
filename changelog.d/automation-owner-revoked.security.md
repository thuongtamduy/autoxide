- Automation rules pause (reason `owner_access_revoked`) when the account they run as is locked out, recovered by
  email, has every session revoked with a password change, or is disabled or deleted; a person with the right
  permissions must check and enable them again. Rules created, edited or enabled through a signed-in session can no
  longer do more than their author could at that time, even if the account gains permissions later (rules created
  with an API key already worked this way).
- New kernel event contract `autoxide_kernel::accounts::ACCESS_REVOKED` (`auth.access_revoked`), emitted by `auth`
  whenever every way into an account is cut.
