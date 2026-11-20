- Module `compliance`: two KYC callbacks for the same attempt processed at once record the result
  and emit `compliance.kyc_verified` once. Adding tracking tags at the same time bumps the cookie
  policy version for every new provider (visitors are asked again), and creating the banner twice
  at once no longer fails with `500` (Postgres).
