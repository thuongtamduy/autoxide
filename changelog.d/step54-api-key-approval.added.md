- `auth`: the tenant setting `auth.api_keys.require_approval` makes new API keys wait (`status: pending`, requests
  get 401 `auth.api_key_pending`) until someone with the new permission `auth.api_keys.approve` approves them with
  `POST /api/v1/auth/all-api-keys/{id}/approve`; keys made by approvers work at once, rotated keys keep their state.
  `GET /api/v1/auth/all-api-keys` filters by `status` and is open to approvers for pending keys. Keys show `status`,
  `approved_by` and `approved_at`.
