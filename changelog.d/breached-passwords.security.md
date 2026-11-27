- Passwords follow NIST SP 800-63B-4: a new password must be 15 to 128 characters (was 12) and must
  not be on a list of about 56,000 common or breached passwords (15 characters or more) that ships
  inside the binary (checked offline and case-insensitively). Every way of setting a password checks it: sign-up,
  administrator and `autoxide user create`, first password, change, reset and accepting an
  invitation. A refused password gets `422` `validation_failed` with field code `common_password`.
  **Upgrade note:** existing passwords keep working, the rules apply only when a password is set;
  clients that check the length themselves should use 15, and a tenant's
  `auth.password_min_length` of 12 to 14 now counts as 15 (it is still shown as stored). A
  bootstrap administrator password (`AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD` or `…_FILE`) and scripts
  that call `autoxide user create` must follow the new rules too: with a password of 12 to 14
  characters, or one on the list, `autoxide serve` stops at startup because it cannot create the
  first administrator, and `autoxide user create` refuses the account.
- `Strict-Transport-Security` is on by default in production: when `AUTOXIDE_HTTP_PUBLIC_URL`
  starts with `https://` and `AUTOXIDE_HTTP_HSTS_SECONDS` is unset, responses carry
  `max-age=31536000`. Development (`http://` or no public URL) sends none. **Upgrade note:** set
  `AUTOXIDE_HTTP_HSTS_SECONDS=0` to keep it off, for example while some clients still reach this
  host over plain HTTP.
