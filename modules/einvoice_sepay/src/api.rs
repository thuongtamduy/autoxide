//! Public contract of the `einvoice_sepay` connector: only the provider name. All work goes through the
//! kernel's `EInvoiceProvider` trait; other modules do not call into here directly.

/// Stable name of the provider: the `einvoice_provider` value of the document series in `invoices`.
pub const PROVIDER: &str = "sepay";
