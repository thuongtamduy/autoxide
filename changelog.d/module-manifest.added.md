- Rust API for modules: a module can declare itself with `Module::manifest` (a `Manifest` built with one function
  per concept: migrations, records, permissions, events, translations, limits…; experimental concepts under
  `Manifest::unstable`) and register its routes, MCP tools, subscriptions, jobs, tasks and checks on `StartContext`
  in `start` (`subscribe`, `routes`, `tools`, `job`…). `StartContext::kernel::<H>()` hands out any kernel handle
  (`Outbox`, `SyncLog`, `EntitlementsHandle`…). The old `Module` functions keep working; the sample module `notes`
  shows the new way.
