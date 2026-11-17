//! Nhà cung cấp hoá đơn điện tử giả cho test: nhận mọi hoá đơn, mặc định cơ quan thuế chấp nhận,
//! cho giả lập từ chối và không trả lời.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use autoxide_kernel::einvoice::{
    EInvoice, EInvoiceCancel, EInvoiceCapabilities, EInvoiceFormat, EInvoiceKind, EInvoiceProvider,
    EInvoiceStatus, IssuedEInvoice, provider_unavailable,
};
use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::TenantId;

/// Tên của nhà cung cấp giả.
pub const FAKE_EINVOICE_PROVIDER: &str = "fake";

/// Nhà cung cấp hoá đơn điện tử giả. Góp vào app bằng
/// `TestApp::builder(…).contribute::<dyn EInvoiceProvider>(provider.clone())`.
#[derive(Default)]
pub struct FakeEInvoiceProvider {
    /// Hoá đơn đã nhận, theo thứ tự (gửi lại sau khi sửa thì có thêm một bản).
    pub issued: Mutex<Vec<EInvoice>>,
    /// Hoá đơn đã huỷ (mã ở nhà cung cấp).
    pub cancelled: Mutex<Vec<String>>,
    /// Lý do từ chối cho lần kiểm trạng thái kế tiếp của từng hoá đơn.
    rejections: Mutex<HashMap<String, String>>,
    reject_next: Mutex<Option<String>>,
    unavailable: AtomicUsize,
}

impl FakeEInvoiceProvider {
    pub fn new() -> Self {
        FakeEInvoiceProvider::default()
    }

    /// Hoá đơn gửi kế tiếp bị cơ quan thuế từ chối với lý do `reason`.
    pub fn reject_next(&self, reason: &str) {
        if let Ok(mut slot) = self.reject_next.lock() {
            *slot = Some(reason.to_owned());
        }
    }

    /// `count` lời gọi kế tiếp lỗi như khi nhà cung cấp không trả lời.
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
        if let Some(reason) = self.reject_next.lock().ok().and_then(|mut slot| slot.take())
            && let Ok(mut rejections) = self.rejections.lock()
        {
            rejections.insert(reference.clone(), reason);
        }
        Ok(IssuedEInvoice::new(reference.clone())
            .lookup_code(format!("LOOKUP-{}", &reference[9..17])))
    }

    async fn status(&self, _tenant: TenantId, provider_ref: &str) -> Result<EInvoiceStatus, Error> {
        self.take_failure()?;
        let rejected = self.rejections.lock().ok().and_then(|mut found| found.remove(provider_ref));
        Ok(match rejected {
            Some(reason) => EInvoiceStatus::Rejected { reason },
            None => EInvoiceStatus::Accepted {
                authority_code: Some(format!("CQT-{}", provider_ref.len())),
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
