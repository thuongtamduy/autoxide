- SMS templates per tenant: the text of every SMS that carries a one-time code can be set by the tenant to
  match the template it registered with the carriers for its SMS brandname — tenant settings
  `compliance.sms_template.phone_verification`, `compliance.sms_template.sign_in`,
  `orders.sms_template.order_lookup`, `orders.sms_template.address_change` and
  `bookings.sms_template.guest_code` (group `sms_template`, `PUT /api/v1/kernel/settings/{name}`). A template
  must contain `{code}` exactly once and only the placeholders of that message; unset, the previous text is
  sent. Kernel: `autoxide_kernel::sms::{check_sms_template, fill_sms_template, tenant_sms_text,
  SMS_TEMPLATE_GROUP}` for modules that send codes.
