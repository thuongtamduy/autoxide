//! Hợp đồng công khai của bộ nối `einvoice_sepay`: chỉ có tên nhà cung cấp. Mọi việc đi qua trait
//! `EInvoiceProvider` của kernel; module khác không gọi thẳng vào đây.

/// Tên ổn định của nhà cung cấp: giá trị `einvoice_provider` của ký hiệu chứng từ ở `invoices`.
pub const PROVIDER: &str = "sepay";
