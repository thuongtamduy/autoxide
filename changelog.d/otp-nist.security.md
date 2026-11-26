- Module `compliance`: SMS sign-in codes follow NIST SP 800-63B-4.
  - A code is valid for at most 10 minutes: `AUTOXIDE_COMPLIANCE_OTP_TTL_SECONDS` now accepts 60 to 600, and a
    larger value is refused at start.
  - After 100 wrong codes in a row for a number, from any clients and with no time window, sign-in codes for it are
    locked (`403 compliance.otp_locked`) until the owner verifies the number again from a signed-in account.
  - A right code resets the count and closes the number's other open codes.
  - Codes per hour are counted per number and client, so strangers using up their own share no longer stop the owner
    from receiving codes. A new cap for all clients together, `AUTOXIDE_COMPLIANCE_OTP_PER_PHONE_ALL_CLIENTS_PER_HOUR`
    (default 10), stops SMS floods to one number.
  - The daily cap on wrong codes for all clients together is now its own setting,
    `AUTOXIDE_COMPLIANCE_OTP_WRONG_PER_PHONE_ALL_CLIENTS_PER_DAY` (default 20, at most 100), instead of five times the
    per-client limit.
  - A client (one IPv4 address, or one IPv6 /64) that signed in with the number in the last 90 days is not counted
    against either all-clients cap.
