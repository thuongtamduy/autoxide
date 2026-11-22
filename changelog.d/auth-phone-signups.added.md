- Module `auth`: `POST /api/v1/auth/phone-signups` creates a buyer account from a verified order lookup
  token (no email, no password) and signs in; later sign-ins use an SMS code or a passkey.
- Kernel: `StartContext::later` gives a module a handle to a contract that a module registered after
  it provides; `autoxide_kernel::sms::VerifiedPhoneTokens` is the neutral contract for tokens issued
  after a one-time code to a phone number.
