- Module `auth`: failed sign-ins are counted per email and client address, with a wider limit for
  the email as a whole, so one client can no longer lock someone else's account. Wrong current
  passwords and second-factor codes are counted per account.
- Module `auth`: customers confirm their current password against their own account kind.
- Module `auth`: using a sign-in link on an account whose email was never verified verifies it and
  removes any password, session, passkey, API key, authenticator app and outside sign-in set up
  before.
- Module `auth`: a passkey's signature counter is updated with a compare-and-swap.
- Module `auth`: a multi-organization Microsoft Entra provider that allows sign-up must list
  `allowed_email_domains`.
