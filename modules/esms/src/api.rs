//! Hợp đồng của module `esms` cho module khác: chỉ có tên nhà cung cấp. Việc gửi tin đi qua trait
//! `autoxide_kernel::sms::SmsSender` mà module này công bố.

/// Tên nhà cung cấp, như `SmsSender::provider` trả về.
pub const PROVIDER: &str = "esms";
