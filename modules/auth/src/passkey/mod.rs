//! Passkey (WebAuthn): đăng nhập bằng vân tay, Face ID, khoá bảo mật thay cho mật khẩu
//! (D-077).

pub(crate) mod authenticators;
pub(crate) mod entity;
mod service;
mod webauthn;

pub(crate) use entity::passkey::Model as Passkey;

/// RP ID theo khách (mục 3 của đợt cấu hình theo khách, D-20261126-tenant-passkey-rp): một tên miền khách đã xác minh.
pub(crate) const TENANT_RP_ID: autoxide_kernel::tenancy::TenantSetting =
    autoxide_kernel::tenancy::TenantSetting::own_domain(
        "auth.passkey_rp_id",
        "Domain this tenant's passkeys belong to (the WebAuthn relying party id), instead of the \
         server's AUTOXIDE_AUTH_PASSKEY_RP_ID: one of this tenant's verified domains, e.g. \
         `shop.example`; passkeys then work on `https://shop.example` only. Checked when set and \
         at every use: if the domain stops being verified for this tenant (or moves to another \
         tenant), the server's passkey setting applies again and passkeys registered on the \
         domain stop working. Changing it makes the tenant's registered passkeys unusable.",
    );
pub(crate) use service::{
    Ceremony, PasskeyService, drop_ceremonies_in, remove_all_in, still_registered_in,
};

#[cfg(test)]
mod tenant_tests;
#[cfg(test)]
pub(crate) mod testing;
#[cfg(test)]
mod tests;
