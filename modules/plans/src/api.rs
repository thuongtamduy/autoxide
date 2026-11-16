//! Hợp đồng của module `plans` cho module khác.
//!
//! Module khác KHÔNG gọi `plans` trực tiếp: quyền dùng đi qua
//! `autoxide_kernel::entitlement::EntitlementsHandle` (lấy bằng `StartContext::entitlements`),
//! module `plans` chỉ là bên cung cấp. Phần cấp phát khách mới (`Provisioning`) thêm vào đây ở mục
//! 5 của bước 35.
