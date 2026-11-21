- `mail`: a tenant's own SMTP server (`mail.connection.smtp.url`) must be an `smtp://` or `smtps://` address on the
  public Internet; it is checked when saved and again when connecting. New setting
  `AUTOXIDE_MAIL_ALLOW_PRIVATE_NETWORKS` (default `false`) allows private networks. Connection errors shown to the
  tenant are general.
