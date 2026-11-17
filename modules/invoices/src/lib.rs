//! Module `invoices` (bước 36 mục 6 và 8, D-344): chứng từ kế toán. Ký hiệu theo nút / loại / năm;
//! chứng từ nháp không có số, thuế tính qua `dyn TaxEngine`; phát hành cấp số tuần tự không nhảy số
//! (`numbering`), ghi sổ qua `dyn Ledger`, gửi hoá đơn điện tử qua `EInvoiceProvider` (kể cả hoá đơn từ
//! máy tính tiền, lập thay người bán khi có uỷ nhiệm); điều chỉnh, thay thế, huỷ. Chứng từ đã phát
//! hành không xoá được (lưu ≥ 10 năm). Module khác lập chứng từ qua [`api::Invoices`].

pub mod api;
mod documents;
mod entity;
mod events;
mod http;
mod http_documents;
mod issue;
mod migration;
mod permissions;
mod provider;
mod service;
mod tools;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use autoxide_kernel::access::Permission;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::EventInfo;
use autoxide_kernel::job::{JobRegistration, Schedule};
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use utoipa_axum::router::OpenApiRouter;

use crate::service::InvoicesService;

pub fn module() -> impl Module {
    InvoicesModule::default()
}

#[derive(Default)]
struct InvoicesModule {
    service: Option<Arc<InvoicesService>>,
}

impl Module for InvoicesModule {
    fn name(&self) -> &'static str {
        "invoices"
    }

    // Module nền: không gói nào tắt được (bước 35, D-323).
    fn always_enabled(&self) -> bool {
        true
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        migration::all()
    }

    // Chứng từ kế toán lưu ít nhất 10 năm, kể cả khi xoá hẳn khách (Luật Kế toán 88/2015, Điều 41).
    fn retained_tables(&self) -> Vec<&'static str> {
        vec!["invoices_series", "invoices_documents", "invoices_lines", "invoices_mandates"]
    }

    fn permissions(&self) -> Vec<Permission> {
        permissions::all()
    }

    fn events(&self) -> Vec<EventInfo> {
        events::all()
    }

    fn translations(&self) -> Vec<(&'static str, &'static str)> {
        vec![("vi", include_str!("../locales/vi.json"))]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let service = Arc::new(InvoicesService {
            databases: context.tenancy(),
            outbox: context.outbox()?,
            jobs: context.jobs(),
            tax: context.require::<dyn autoxide_tax::api::TaxEngine>()?,
            ledger: context.require::<dyn autoxide_ledger::api::Ledger>()?,
            providers: context.contributions::<dyn autoxide_kernel::einvoice::EInvoiceProvider>(),
        });
        self.service = Some(service.clone());
        context.provide::<dyn api::Invoices>(Arc::new(provider::Provider(service)))
    }

    fn routes(&self) -> OpenApiRouter {
        self.service.clone().map_or_else(OpenApiRouter::new, |service| {
            http::routes(service.clone()).merge(http_documents::routes(service))
        })
    }

    fn tools(&self) -> Vec<Tool> {
        self.service.as_ref().map_or_else(Vec::new, tools::all)
    }

    fn jobs(&self) -> Vec<JobRegistration> {
        self.service.clone().map_or_else(Vec::new, |service| {
            vec![
                JobRegistration::new(
                    issue::SUBMIT_EINVOICE,
                    Arc::new(issue::SubmitJob(service.clone())),
                ),
                JobRegistration::new(issue::POLL_EINVOICES, Arc::new(issue::PollJob(service))),
            ]
        })
    }

    fn schedules(&self) -> Vec<Schedule> {
        vec![Schedule::new(
            issue::POLL_EINVOICES,
            issue::POLL_EINVOICES,
            std::time::Duration::from_secs(300),
        )]
    }
}
