- Kernel: `ReferenceSource::referrers_page` (one page of the records of a module that point at a record, each with
  its key, `KeyedReferrer`) with a default implementation, so sources written before it keep compiling; the default
  reads at most `REFERRERS_PAGE_FALLBACK_LIMIT` referrers through `referrers` and cuts the page in memory — override
  it with a keyset page in your database.
- Setting `AUTOXIDE_COMPLIANCE_PURPOSES_TENANT_MAX`: most processing purposes of its own a tenant may add (default
  200).
