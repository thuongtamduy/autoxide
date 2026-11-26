- Event `mail.delivery_failed` carries `enhanced` (the SMTP enhanced status) and `hard` (the address
  itself does not work); the newsletter suppresses a subscriber only when `hard` is true, so a sending
  misconfiguration no longer empties its lists.
