- New experimental module `esms`: sends one-time codes and customer-care SMS through eSMS (Vietnam) under each
  tenant's own account (`esms.connection.sms.api_key`, `.secret_key`, `.brandname`, `.sandbox`), with a connection
  test. Advertising messages are refused. Off by default; turn on with `AUTOXIDE_MODULES=official,esms`. Written
  from the provider's public documentation; not yet verified against the live service. Guide:
  `docs/guides/providers/esms.md`.
