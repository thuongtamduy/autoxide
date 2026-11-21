- Tenant data export (`/api/v1/kernel/tenants/{id}/exports`) writes date columns as `YYYY-MM-DD`; they were
  exported as `null`, so the dates were missing from the file.
