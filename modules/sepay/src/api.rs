//! Hợp đồng công khai của bộ nối `sepay`: chỉ có tên bộ nối. Mọi việc đi qua trait `BankFeed` của
//! kernel; module khác không gọi thẳng vào đây.

/// Tên ổn định của bộ nối: giá trị `feed_provider` của tài khoản ngân hàng ở `ledger` và đoạn
/// `{provider}` trong đường webhook `/api/v1/ledger/bank-feeds/{provider}/{hook}`.
pub const PROVIDER: &str = "sepay";
