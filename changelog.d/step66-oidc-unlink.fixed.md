- Module `auth`: removing two sign-in links of an account at the same time can no longer remove
  both and lock the owner out; the "last way to sign in" check now runs inside the transaction,
  after locking the account row.
