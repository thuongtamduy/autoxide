- Module `compliance`: a late result of an old identity check no longer verifies a profile again
  after it was revoked, approved by hand, or moved on to a newer check; that attempt is kept as
  `superseded`.
- Module `compliance`: two profiles with the same identity document verified at the same time no
  longer both become `verified`; the later one waits for a reviewer (`duplicate_document`). New
  migration `m0020_kyc_verified_document`.
- Module `compliance`: when two cases that hid the same target are closed at the same time, the
  target is shown again (`compliance.target_restored`) instead of staying hidden.
