//! Chính sách tách / gộp khách của các bảng của `auth` (bước 38, D-20261118-step38-relocation).
//!
//! - Membership (`auth_user_role`), API key, lời mời gắn nút đi theo nút.
//! - Tài khoản có membership ở nút trong nhánh được CHÉP sang khách mới (cùng ID, cùng mật khẩu,
//!   TOTP, passkey) — khách nguồn vẫn giữ tài khoản đó; phiên đăng nhập không chép: người đó đăng
//!   nhập lại ở khách mới.
//! - Role là cấu hình của khách: khách mới bắt đầu bằng bản sao; gộp thì role cùng ID bỏ qua.

use autoxide_kernel::tenancy::Relocation;

pub(crate) fn relocation() -> Relocation {
    Relocation::new()
        .referenced("auth_user", "auth_user_role", "user_id")
        .copy_on_split(&["auth_role", "auth_oidc_provider"])
        .follow("auth_role_permission", "role_id", "auth_role")
        .by_org(&["auth_api_key", "auth_user_role", "auth_invitation"])
        .follow("auth_api_key_permission", "api_key_id", "auth_api_key")
        .follow("auth_totp", "user_id", "auth_user")
        .follow("auth_recovery_code", "user_id", "auth_user")
        .follow("auth_passkey", "user_id", "auth_user")
        .follow("auth_identity", "user_id", "auth_user")
        // Uỷ quyền cấp cho khách con là của khách nguồn.
        .stay(&["auth_delegation"])
        .ephemeral(&[
            "auth_throttle",
            "auth_tokens",
            "auth_refresh_token",
            "auth_mfa_challenge",
            "auth_passkey_ceremony",
            "auth_support_grant",
        ])
        .sealed("auth_totp", "secret", "auth.totp_secret", "id")
        .sealed("auth_oidc_provider", "client_secret", "auth.oidc_client_secret", "id")
}
