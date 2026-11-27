- OAuth: a code exchange or refresh that runs while the same approval is being revoked (another use of the same refresh
  token, or the person revoking) no longer issues new tokens after the revocation; it answers `invalid_grant`, and no
  token of a revoked approval is kept.
