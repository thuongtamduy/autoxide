- Module `compliance`: SMS sign-in codes follow NIST SP 800-63B-4.
  - A code is valid for at most 10 minutes: `AUTOXIDE_COMPLIANCE_OTP_TTL_SECONDS` now accepts 60 to 600, and a
    larger value is refused at start.
  - Wrong codes in a row for a number are counted with no time window, and a right code resets the count. From 90
    wrong codes the number is locked (`403 compliance.otp_locked`) for clients that never signed in with it; from 100,
    for every client.
  - A locked number is unlocked when its owner verifies it again from a signed-in account, or by an administrator.
    The administrator path uses the new permission `compliance.phone_locks.manage`,
    `POST /api/v1/compliance/phone-locks/unlock`, the MCP tool `compliance.unlock_phone`, and
    `autoxide task compliance.unlock_phone`. Each unlock is written to the audit log.
  - A right code closes the number's other open sign-in codes.
  - Codes per hour are counted per number and client, so strangers using up their share no longer stop the owner from
    receiving codes.
  - New caps against SMS floods: `AUTOXIDE_COMPLIANCE_OTP_PER_PHONE_ALL_CLIENTS_PER_HOUR` (default 5, all clients) and
    `AUTOXIDE_COMPLIANCE_OTP_PER_PHONE_PER_DAY` (default 20, shared with phone verification codes).
  - The daily cap on wrong codes for all clients is its own setting,
    `AUTOXIDE_COMPLIANCE_OTP_WRONG_PER_PHONE_ALL_CLIENTS_PER_DAY` (default 20, at most 100), instead of five times the
    per-client limit.
  - A client (one IPv4 address, or one IPv6 /64) that signed in with the number in the last 90 days does not count
    toward the all-clients caps.
