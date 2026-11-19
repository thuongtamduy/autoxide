- `auth`: guardrails for child tenants. A tenant above another sets the most permissions it can use
  (`PUT /api/v1/auth/guardrails/{tenant_id}`, permission `auth.guardrails.manage`); no role there or in tenants below
  it, not even administrators or people entering by delegation, gets more, and refusals carry
  `reason: outside_guardrail`. Guardrails from several tenants above all apply. `GET /api/v1/auth/guardrails` lists
  the ones a tenant set, `GET /api/v1/auth/guardrails/applied` the ones limiting it.
