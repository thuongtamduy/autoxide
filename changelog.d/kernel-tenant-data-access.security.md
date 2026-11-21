- Kernel: exporting the data of another tenant (`/api/v1/kernel/tenants/{id}/exports…`, MCP `export_tenant`) and
  making a sandbox of it now need access to that tenant (header `Autoxide-Tenant` under a support access grant in
  force); requests, downloads and download links of an export are recorded in the tenant's own events
  (`kernel.tenant_export_requested`, `kernel.tenant_export_accessed`). New codes
  `kernel.export_needs_tenant_access`, `tenancy.sandbox_needs_tenant_access`.
