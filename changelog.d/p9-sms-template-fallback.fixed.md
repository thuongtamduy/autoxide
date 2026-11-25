- SMS templates per tenant: when the tenant's template cannot be read (database error), the one-time code is
  sent with the default text (and a warning is logged) instead of failing after the code was stored.
  `autoxide_kernel::sms::tenant_sms_text` now returns `Option<String>`.
