- Module `compliance`: identity profiles keep the seller items of Vietnam's Decree 248/2026 Article 18 (date of birth,
  head office, legal representative, issuing country and end date of the document); a reviewer records them with
  `PUT /api/v1/compliance/kyc/profiles/{id}/details`. From 2027-01-01 (rule `kyc.required_activities` version 2) a
  seller missing one cannot sell (`compliance.kyc_required`, `profile_status` `incomplete`, `missing`); a passport must
  stay valid 6 months after the verification; a profile never stays verified past its document's end date.
- Module `catalog`: on a marketplace, goods of a seller whose verification expired or was revoked go `inactive`; a
  daily job `catalog.seller_kyc_sweep` and a branch move check sellers again (a tenant that became a marketplace, a
  rule that started to apply).
- Module `compliance`: data subject requests get `fulfil_due_at`, the deadline to carry them out by kind (rule
  `privacy.subject_requests` version 3, calendar days); `overdue` now means still `received` past `due_at`, or not
  done past `fulfil_due_at`.
- Module `tax`: pack `vn` version 4 after Decree 68/2026 (household business thresholds: no VAT or PIT up to 500
  million VND a year, e-invoices not mandatory under it); `TaxEngine::invoice_advice` says which invoice a sale needs.
