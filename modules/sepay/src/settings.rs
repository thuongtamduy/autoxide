//! Tài khoản SePay của một khách: cấu hình nhóm `connection`, khách tự đặt qua
//! `PUT /api/v1/kernel/settings/{name}`. Trường bí mật không bao giờ được trả lại.

use autoxide_kernel::tenancy::{CONNECTION_GROUP, TenantSetting};

/// API token của SePay API v2 (đọc giao dịch để đối soát, thử kết nối).
pub(crate) const API_TOKEN: TenantSetting = TenantSetting::new(
    "sepay.connection.bank.api_token",
    "API token of this tenant's SePay company (my.sepay.vn > Company settings > API Keys). Used \
     to read bank transactions that a webhook missed and to test the connection.",
)
.secret()
.group(CONNECTION_GROUP);

/// Secret Key của webhook khi chọn xác thực HMAC-SHA256.
pub(crate) const WEBHOOK_SECRET: TenantSetting = TenantSetting::new(
    "sepay.connection.bank.webhook_secret",
    "Secret Key of this tenant's SePay webhook when it uses HMAC-SHA256 authentication \
     (recommended). When set, every notification must carry a valid `X-SePay-Signature` no \
     older than 5 minutes.",
)
.secret()
.group(CONNECTION_GROUP);

/// API Key của webhook khi chọn xác thực API Key.
pub(crate) const WEBHOOK_API_KEY: TenantSetting = TenantSetting::new(
    "sepay.connection.bank.webhook_api_key",
    "API Key of this tenant's SePay webhook when it uses API Key authentication. Used only when \
     `sepay.connection.bank.webhook_secret` is not set; HMAC-SHA256 is safer.",
)
.secret()
.group(CONNECTION_GROUP);

pub(crate) fn all() -> Vec<TenantSetting> {
    vec![API_TOKEN, WEBHOOK_SECRET, WEBHOOK_API_KEY]
}
