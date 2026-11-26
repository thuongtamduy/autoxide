- Automation rules pause (reason `owner_access_revoked`) when the account they run as is locked out, recovered by
  email, has every session revoked with a password change, or is disabled or deleted; a person with the right
  permissions must check and enable them again. Whenever a rule stops being `active` (any pause, disabled), its runs
  still waiting end with `automations.rule_not_active`; a run that started before the rule was edited or enabled again
  stops with `automations.rule_changed` instead of finishing old actions under the new person responsible. This also
  happens when the owner changes their password and signs out everywhere. Rules created, edited or enabled through a
  signed-in session can no longer use a permission their author did not have at that time, even if the account gains
  it later (the limit is by permission name; rules created with an API key already worked this way).
  **Action needed:** rules saved before this release have no such limit until someone edits or enables them again;
  review and re-enable the rules that matter.
- New kernel event contract `autoxide_kernel::accounts::ACCESS_REVOKED` (`auth.access_revoked`), emitted by `auth`
  whenever every way into an account is cut.
