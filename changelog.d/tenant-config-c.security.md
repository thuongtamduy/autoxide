- The `kernel.api_requests` plan quota counts only signed-in callers, against the tenant they act in: an
  anonymous request naming a tenant (`Autoxide-Tenant`, `Host`) no longer uses up that tenant's quota,
  and once a tenant's quota is used up, public routes, inbound webhooks, platform staff and support
  access still get through. A request refused with 429 no longer uses up the quota.
