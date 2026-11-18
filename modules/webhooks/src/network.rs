//! Chặn webhook gọi vào mạng nội bộ (SSRF, D-149). Bộ lọc dùng chung ở
//! [`autoxide_kernel::net`]; ở đây chỉ có câu giải thích riêng của webhook.

pub(crate) use autoxide_kernel::net::{PrivateAddress, PublicOnly, is_public_host};

/// Câu giải thích khi địa chỉ nằm trong mạng riêng.
pub(crate) const PRIVATE_ADDRESS: &str = "points to a private or local network address; \
     webhooks are sent only to public addresses unless the server sets \
     `AUTOXIDE_WEBHOOKS_ALLOW_PRIVATE_NETWORKS=true`";
