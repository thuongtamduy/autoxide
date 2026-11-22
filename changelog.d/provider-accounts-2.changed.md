- Setting or removing a tenant setting of the `connection` group (a partner account) now needs a recent sign-in:
  `PUT` / `DELETE /api/v1/kernel/settings/{name}` answer `401 step_up_required` otherwise (API keys are not asked).
  For adapter authors: `ProviderAccounts::webhook_url`, `provider::Environment` (sandbox accounts per tenant) and
  `Principal::require_fresh`.
