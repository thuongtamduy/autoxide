//! Link trong thư của `auth` theo từng khách (D-20261125-tenant-link-templates): khách đặt trang riêng (đường
//! dẫn tuỳ ý, vd `/dat-lai-mat-khau`) trên host của mình; chưa đặt thì dùng biến chung của máy chủ, đổi host
//! theo `kernel.public_url`. Kiểm host và placeholder do kernel làm (`SiteLinks::link`).

use autoxide_kernel::tenancy::TenantSetting;

pub(crate) const PASSWORD_RESET: TenantSetting = TenantSetting::link(
    "auth.link.password_reset",
    "Page of this tenant's app where people choose a new password, such as \
     `https://shop.example/dat-lai-mat-khau?token={token}`. Must be on this tenant's own host \
     (verified domain or `kernel.public_url`). When unset, the server's \
     AUTOXIDE_AUTH_PASSWORD_RESET_URL is used.",
    &["{token}"],
);

pub(crate) const EMAIL_VERIFICATION: TenantSetting = TenantSetting::link(
    "auth.link.email_verification",
    "Page of this tenant's app that confirms an email address, such as \
     `https://shop.example/xac-minh-email?token={token}`. Must be on this tenant's own host. When \
     unset, the server's AUTOXIDE_AUTH_EMAIL_VERIFICATION_URL is used.",
    &["{token}"],
);

pub(crate) const INVITATION: TenantSetting = TenantSetting::link(
    "auth.link.invitation",
    "Page of this tenant's app that accepts an invitation, such as \
     `https://shop.example/loi-moi?token={token}`. Must be on this tenant's own host. When unset, \
     the server's AUTOXIDE_AUTH_INVITATION_URL is used.",
    &["{token}"],
);

pub(crate) const LOGIN_LINK: TenantSetting = TenantSetting::link(
    "auth.link.login",
    "Page of this tenant's app that signs in with a link, such as \
     `https://shop.example/dang-nhap/link?token={token}`. Must be on this tenant's own host. When \
     unset, the server's AUTOXIDE_AUTH_LOGIN_LINK_URL is used.",
    &["{token}"],
);

pub(crate) const EMAIL_CHANGE_REPORT: TenantSetting = TenantSetting::link(
    "auth.link.email_change_report",
    "Page of this tenant's app where the owner of an account reports an email change they did not \
     make, such as `https://shop.example/bao-doi-email?token={token}`. Must be on this tenant's own \
     host. When unset, the server's AUTOXIDE_AUTH_EMAIL_CHANGE_REPORT_URL is used.",
    &["{token}"],
);

/// Mọi link của `auth` (khai qua `Module::tenant_settings`).
pub(crate) const ALL: [TenantSetting; 5] =
    [PASSWORD_RESET, EMAIL_VERIFICATION, INVITATION, LOGIN_LINK, EMAIL_CHANGE_REPORT];
