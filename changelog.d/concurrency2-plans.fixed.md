- Module `plans`: voiding a billing period while its invoice is being issued now also cancels that
  invoice; a voided period never keeps a live VAT invoice.
- Module `plans`: the daily usage reconciliation no longer overwrites limit holds and releases made
  while it counts, so a tenant cannot go over a limit because of it; holds that have expired but
  are not yet released stay counted until the release job returns them.
