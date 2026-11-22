//! Hợp đồng của module `momo`: không có trait riêng. Module góp một `dyn PaymentGateway`
//! (`autoxide_kernel::payment`) cho `ledger`; module khác chỉ cần tên đối tác để chọn cổng.

/// Tên đối tác: giá trị `provider` của một thanh toán và đoạn trong đường dẫn webhook
/// (`/api/v1/ledger/webhooks/momo/{hook}`).
pub const PROVIDER: &str = "momo";
