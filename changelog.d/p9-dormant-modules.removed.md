- Error code `tenant_move_modules_off` (409 on moving, splitting or merging a tenant that has data of a
  turned-off module): the situation no longer exists, since the tables of turned-off modules now travel with
  the tenant. Clients that handled this code can drop that branch.
