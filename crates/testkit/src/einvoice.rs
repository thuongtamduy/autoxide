//! A fake e-invoice provider for tests: accepts every invoice, the tax authority accepts by default,
//! and it can simulate rejection and no response.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use autoxide_kernel::einvoice::{
    EInvoice, EInvoiceCancel, EInvoiceCapabilities, EInvoiceFormat, EInvoiceKind, EInvoiceProvider,
    EInvoiceStatus, IssuedEInvoice, provider_unavailable,
};
use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::TenantId;

/// Name of the fake provider.
pub const FAKE_EINVOICE_PROVIDER: &str = "fake";

/// A fake e-invoice provider. Contribute it to the app with
/// `TestApp::builder(…).contribute::<dyn EInvoiceProvider>(provider.clone())`.
#[derive(Default)]
pub struct FakeEInvoiceProvider {
    /// Invoices received, in order (resending after a fix adds another copy).
    pub issued: Mutex<Vec<EInvoice>>,
    /// Cancelled invoices (the provider-side code).
    pub cancelled: Mutex<Vec<String>>,
    /// Rejection reason of the most recent submission of each invoice.
    rejections: Mutex<HashMap<String, String>>,
    reject_next: Mutex<Option<String>>,
    unavailable: AtomicUsize,
}

impl FakeEInvoiceProvider {
    pub fn new() -> Self {
        FakeEInvoiceProvider::default()
    }

    /// The next invoice submitted is rejected by the tax authority with the reason `reason`.
    pub fn reject_next(&self, reason: &str) {
        if let Ok(mut slot) = self.reject_next.lock() {
            *slot = Some(reason.to_owned());
        }
    }

    /// The next `count` calls fail as if the provider did not respond.
    pub fn fail_next(&self, count: usize) {
        self.unavailable.store(count, Ordering::SeqCst);
    }

    fn take_failure(&self) -> Result<(), Error> {
        let failed = self
            .unavailable
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |left| left.checked_sub(1))
            .is_ok();
        if failed { Err(provider_unavailable()) } else { Ok(()) }
    }
}

#[async_trait::async_trait]
impl EInvoiceProvider for FakeEInvoiceProvider {
    fn provider(&self) -> &'static str {
        FAKE_EINVOICE_PROVIDER
    }

    fn capabilities(&self) -> EInvoiceCapabilities {
        EInvoiceCapabilities::new(&[
            EInvoiceKind::Vat,
            EInvoiceKind::Sales,
            EInvoiceKind::CashRegister,
            EInvoiceKind::WithholdingCertificate,
        ])
        .with_corrections()
    }

    async fn issue(&self, _tenant: TenantId, invoice: &EInvoice) -> Result<IssuedEInvoice, Error> {
        self.take_failure()?;
        let reference = format!("fake_inv_{}", invoice.document_id.simple());
        if let Ok(mut issued) = self.issued.lock() {
            issued.push(invoice.clone());
        }
        // Kết quả gắn với lần gửi này: gửi lại (sau khi sửa) thì thay kết quả cũ. Hỏi trạng thái không
        // "tiêu" kết quả — hai lần hỏi đồng thời (job gửi và lịch hỏi định kỳ) thấy cùng một câu trả lời.
        let rejection = self.reject_next.lock().ok().and_then(|mut slot| slot.take());
        if let Ok(mut rejections) = self.rejections.lock() {
            match rejection {
                Some(reason) => rejections.insert(reference.clone(), reason),
                None => rejections.remove(&reference),
            };
        }
        Ok(IssuedEInvoice::new(reference.clone())
            .lookup_code(format!("LOOKUP-{}", &reference[9..17])))
    }

    async fn status(&self, _tenant: TenantId, provider_ref: &str) -> Result<EInvoiceStatus, Error> {
        self.take_failure()?;
        let rejected =
            self.rejections.lock().ok().and_then(|found| found.get(provider_ref).cloned());
        Ok(match rejected {
            Some(reason) => EInvoiceStatus::Rejected { reason },
            None => EInvoiceStatus::Accepted {
                authority_code: Some(format!("CQT-{}", provider_ref.len())),
                number: None,
            },
        })
    }

    async fn cancel(&self, _tenant: TenantId, request: &EInvoiceCancel) -> Result<(), Error> {
        self.take_failure()?;
        if let Ok(mut cancelled) = self.cancelled.lock() {
            cancelled.push(request.provider_ref.clone());
        }
        Ok(())
    }

    async fn download(
        &self,
        _tenant: TenantId,
        provider_ref: &str,
        format: EInvoiceFormat,
    ) -> Result<bytes::Bytes, Error> {
        Ok(bytes::Bytes::from(match format {
            EInvoiceFormat::Xml => format!("<invoice ref=\"{provider_ref}\"/>"),
            EInvoiceFormat::Pdf => format!("%PDF-1.7 {provider_ref}"),
        }))
    }
}
