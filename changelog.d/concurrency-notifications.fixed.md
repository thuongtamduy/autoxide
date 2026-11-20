- Module `notifications`: registering the same push endpoint or device token twice at the same
  time keeps a working subscription — the stored secret is always sealed for the row's own ID.
