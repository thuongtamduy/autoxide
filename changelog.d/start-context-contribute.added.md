- Rust API for modules: `StartContext::contribute::<dyn Trait>(…)` is now the one way to plug into the kernel's
  extension points — initial and sample data for new tenants (`TenantSeed`, the new `tenancy::SampleData`), sandbox
  copy (`SandboxCopy`), personal data (`PersonalData`), work inbox, search, summary metrics, list exports, bulk
  actions, code resolvers, setup steps, connection probes, references and the posting verifier. The older
  registration functions (`personal_data`, `work_source`…) and getters (`outbox()`, `sync_log()`…) keep working; new
  code uses `contribute` and `StartContext::kernel::<H>()`. The sample module `notes` shows the new way.
