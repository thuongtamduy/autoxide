//! Tài khoản SePay eInvoice của một khách: cấu hình nhóm `connection`, khách tự đặt qua
//! `PUT /api/v1/kernel/settings/{name}`. Client secret không bao giờ được trả lại.

use autoxide_kernel::tenancy::{CONNECTION_GROUP, TenantSetting};

pub(crate) const CLIENT_ID: TenantSetting = TenantSetting::new(
    "einvoice_sepay.connection.api.client_id",
    "Client id of this tenant's SePay eInvoice account.",
)
.group(CONNECTION_GROUP);

pub(crate) const CLIENT_SECRET: TenantSetting = TenantSetting::new(
    "einvoice_sepay.connection.api.client_secret",
    "Client secret of this tenant's SePay eInvoice account.",
)
.secret()
.group(CONNECTION_GROUP);

pub(crate) const PROVIDER_ACCOUNT_ID: TenantSetting = TenantSetting::new(
    "einvoice_sepay.connection.api.provider_account_id",
    "Id (UUID) of the e-invoice provider account to issue with, from SePay eInvoice \
     (`GET /v1/provider-accounts`). May be left unset when the SePay account has exactly one \
     active provider account.",
)
.group(CONNECTION_GROUP);

pub(crate) fn all() -> Vec<TenantSetting> {
    vec![CLIENT_ID, CLIENT_SECRET, PROVIDER_ACCOUNT_ID]
}
