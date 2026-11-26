- A caller without any of an endpoint's permissions now gets 403 `permission_denied` before anything else is
  checked: before the request body is validated, before `If-Match` is required, and before the record is looked
  up. Callers without access no longer learn which fields are wrong, which endpoints need an `ETag`, or which
  records exist.
- `x-permissions` in the API document now lists every permission an endpoint accepts (18 operations of
  `inventory`, `kernel` tenant exports, `notifications`, `content`, `customers` and `support` were missing one).
