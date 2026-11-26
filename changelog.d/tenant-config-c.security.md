- The `kernel.api_requests` plan quota counts only signed-in callers in their own tenant: an anonymous
  request naming a tenant (`Autoxide-Tenant`, `Host`) no longer uses up that tenant's quota.
