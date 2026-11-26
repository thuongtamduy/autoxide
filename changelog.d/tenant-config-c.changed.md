- Event `mail.delivery_failed` carries `enhanced` (the SMTP enhanced status) and `hard` (the address
  itself does not work); the newsletter suppresses a subscriber only when `hard` is true, so a sending
  misconfiguration no longer empties its lists. An event without `hard` (queued before the upgrade, or
  from a module following the older contract) only marks that mail failed.
