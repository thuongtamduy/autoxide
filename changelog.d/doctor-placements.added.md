- `autoxide doctor` checks the tenant databases (`kernel.placements`): it names every one that is behind (its tenants
  get 503) or was left `migrating` by a process that stopped, and says how to fix it.
