- Module `compliance`: a phone number that entered too many wrong sign-in codes within 24 hours
  (`AUTOXIDE_COMPLIANCE_OTP_WRONG_PER_PHONE_PER_DAY`, default 10, counted across all its codes) gets no
  new code and no code check until the oldest wrong entry is 24 hours old (`compliance.otp_daily_limit`).
