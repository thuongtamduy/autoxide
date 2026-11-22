//! Hợp đồng của bộ nối `ghn` cho module khác: chỉ có mã của hãng (trùng `carrier_code` của tài
//! khoản hãng trong module `shipping`).

/// Mã của hãng Giao Hàng Nhanh trong sổ hãng của `shipping`.
pub const PROVIDER: &str = "ghn";
