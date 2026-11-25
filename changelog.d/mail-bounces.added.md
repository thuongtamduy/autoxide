- Module `mail`: bounce records and a suppression list for all mail, per tenant. A recipient address
  the receiving server refuses for good (SMTP 5.1.x / 5.2.x) is recorded; after
  `AUTOXIDE_MAIL_HARD_BOUNCE_LIMIT` hard bounces it is suppressed for that tenant and new mail to it is
  skipped and counted. `GET /api/v1/mail/bounces` (`?email=`, `?suppressed=`), `/stats`,
  `POST /api/v1/mail/bounces/{id}/release`, `DELETE /api/v1/mail/bounces/{id}`; permissions
  `mail.bounces.read`, `mail.bounces.manage`; records kept `AUTOXIDE_MAIL_BOUNCE_RETENTION_DAYS`.
