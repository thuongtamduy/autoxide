//! Contributions to the kernel's own extension points through [`StartContext::contribute`]
//! (D-20261126-module-manifest, D-20261128-contribute): one registration call for every "many modules plug in
//! an implementation" concept, instead of one function per concept (`work_source`, `personal_data`…).
//!
//! | Trait contributed | Old registration function |
//! | --- | --- |
//! | [`TenantSeed`](crate::tenancy::TenantSeed) | `StartContext::seed_tenant` / `Module::seed_tenant` |
//! | [`SampleData`](crate::tenancy::SampleData) | `StartContext::sample_data` / `Module::sample_data` |
//! | `SandboxCopy` (experimental) | `StartContext::sandbox_copy` / `Module::sandbox_copy` |
//! | [`SwitchHook`](crate::tenancy::SwitchHook) | `Relocation::on_switch` (in `Module::relocation` / `Manifest::relocation`) |
//! | [`PersonalData`](crate::privacy::PersonalData) | `StartContext::personal_data` |
//! | [`PostingVerifier`](crate::access::PostingVerifier) | `StartContext::posting_verifier` |
//! | `WorkSource`, `FinderSource`, `SuggestSource`, `SummaryMetric`, `ExportSource`, `BulkAction`, `CodeResolver`, `SetupStep`, `ConnectionProbe`, `ReferenceSource` | `work_source`, `finder_source`, `suggest_source`, `summary_metric`, `export_source`, `bulk_action`, `code_resolver`, `setup_step`, `connection_probe`, `reference_source` |

// Cài đặt: `contribute::<T>` là hàm generic, `T` biết lúc biên dịch. `Arc<T>` được xem như `dyn Any` rồi thử ép
// về từng `Arc<dyn Trait>` của kernel (`downcast_ref` so `TypeId`, sai kiểu trả `None` — không có gì không an
// toàn). Khớp thì gửi vào đúng registry như hàm đăng ký cũ; không khớp thì là trait giữa các module (cất ở `Apis`).
// Seed, dữ liệu mẫu và bản sao sandbox không có registry dùng được lúc `start`: chúng cất ở `Apis` như đóng góp
// thường, registry gom sau khi mọi module đã khởi động (`registry/contributed.rs`).

use std::any::Any;
use std::sync::Arc;

use crate::error::Error;

use super::StartContext;

impl StartContext<'_> {
    /// Sends `api` to the kernel registry for its trait, as the old registration function did; `None` when the
    /// trait is not one of the kernel's registries.
    pub(super) fn contribute_to_kernel(&mut self, api: &dyn Any) -> Option<Result<(), Error>> {
        let module = self.module;
        // Mỗi dòng: trait của kernel → registry nhận nó (đúng thân của hàm đăng ký cũ).
        macro_rules! route {
            ($($contract:ty => $registry:expr;)+) => {$(
                if let Some(api) = api.downcast_ref::<Arc<$contract>>() {
                    return Some($registry.register(module, Arc::clone(api)));
                }
            )+};
        }
        route! {
            dyn crate::privacy::PersonalData => self.personal_data;
            dyn crate::access::PostingVerifier => self.verification;
            dyn crate::ops::work::WorkSource => self.work;
            dyn crate::ops::find::FinderSource => self.finders;
            dyn crate::ops::suggest::SuggestSource => self.suggestions;
            dyn crate::ops::summary::SummaryMetric => self.metrics;
            dyn crate::ops::list_exports::ExportSource => self.exports;
            dyn crate::ops::bulk::BulkAction => self.bulk;
            dyn crate::ops::resolve::CodeResolver => self.resolvers;
            dyn crate::ops::setup::SetupStep => self.setup;
            dyn crate::ops::connections::ConnectionProbe => self.probes;
        }
        if let Some(api) = api.downcast_ref::<Arc<dyn crate::ops::usages::ReferenceSource>>() {
            self.references.register(module, Arc::clone(api));
            return Some(Ok(()));
        }
        None
    }
}
