- For authors of external modules: `KeyedReferrer` (returned by `ReferenceSource::referrers_page`) is now
  `#[non_exhaustive]`. Build it with `KeyedReferrer::new(key, referrer)` instead of a struct literal, so fields can be
  added later without breaking your module.
