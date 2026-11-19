//! Migration của module `auth`, theo thứ tự áp dụng. Đã commit thì không sửa —
//! thêm file mới và một dòng vào cuối [`all`].

mod m0001_create_user;
mod m0002_create_refresh_token;
mod m0003_create_roles;
mod m0004_create_login_throttle;
mod m0005_create_api_keys;
mod m0006_create_totp;
mod m0007_create_recovery_codes;
mod m0008_create_mfa_challenge;
mod m0011_create_password_reset;
mod m0012_add_email_verified;
mod m0013_create_email_verification;
mod m0014_create_passkeys;
mod m0015_add_session_details;
mod m0016_add_user_search;
mod m0017_fold_user_search;
mod m0018_move_login_throttle;
mod m0019_move_tokens;
mod m0021_add_cleanup_indexes;
mod m0023_add_last_sign_in;
mod m0024_session_sign_in;
mod m0025_passkey_aaguid;
mod m0026_tenancy;
mod m0027_widen_totp_secret;
mod m0029_org_scopes;
mod m0030_account;
mod m0031_realm;
mod m0032_delegations;
mod m0033_invitations;
mod m0034_support_grants;
mod m0035_api_key_kind;
mod m0036_oidc;
mod m0037_role_denies;
mod m0038_api_key_networks;
pub(crate) mod u0001_seal_totp;

use autoxide_kernel::migration::prelude::MigrationTrait;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(m0001_create_user::Migration),
        Box::new(m0002_create_refresh_token::Migration),
        Box::new(m0003_create_roles::Migration),
        Box::new(m0004_create_login_throttle::Migration),
        Box::new(m0005_create_api_keys::Migration),
        Box::new(m0006_create_totp::Migration),
        Box::new(m0007_create_recovery_codes::Migration),
        Box::new(m0008_create_mfa_challenge::Migration),
        // Bảng `auth_events` (transactional outbox), định nghĩa ở kernel.
        autoxide_kernel::event::outbox_migration("m0009_create_events", "auth"),
        // Bảng `auth_jobs` (job queue), định nghĩa ở kernel.
        autoxide_kernel::job::jobs_migration("m0010_create_jobs", "auth"),
        Box::new(m0011_create_password_reset::Migration),
        Box::new(m0012_add_email_verified::Migration),
        Box::new(m0013_create_email_verification::Migration),
        Box::new(m0014_create_passkeys::Migration),
        Box::new(m0015_add_session_details::Migration),
        Box::new(m0016_add_user_search::Migration),
        Box::new(m0017_fold_user_search::Migration),
        Box::new(m0018_move_login_throttle::Migration),
        Box::new(m0019_move_tokens::Migration),
        // Cột `actor_kind` của bảng `auth_events` (người lạ khác hệ thống, D-122).
        autoxide_kernel::event::outbox_actor_migration("m0020_add_event_actor", "auth"),
        Box::new(m0021_add_cleanup_indexes::Migration),
        // Bảng phụ trợ của kernel (event, job, throttle, token, nonce) mang `tenant_id` (D-253).
        autoxide_kernel::tenancy::migration::adopt(
            "m0022_tenancy_helpers",
            vec![
                autoxide_kernel::event::tenant_table("auth"),
                autoxide_kernel::job::tenant_table("auth"),
                autoxide_kernel::throttle::tenant_table("auth"),
                autoxide_kernel::tokens::tenant_table("auth"),
            ],
        ),
        Box::new(m0023_add_last_sign_in::Migration),
        // Cách đăng nhập và quốc gia gần đúng của phiên (D-272).
        Box::new(m0024_session_sign_in::Migration),
        // Loại authenticator của passkey (D-272).
        Box::new(m0025_passkey_aaguid::Migration),
        // Mọi bảng của `auth` thuộc một khách (bước 33 B12).
        Box::new(m0026_tenancy::Migration::new()),
        // Secret TOTP đã mã hoá dài hơn cột cũ (Postgres). Ngay sau đó upgrade `u0001_seal_totp`
        // mã hoá secret đã có.
        Box::new(m0027_widen_totp_secret::Migration),
        // Danh sách theo khách sắp theo `id` cần index `(tenant_id, id)` (đo ở bước 33 C, D-302).
        autoxide_kernel::tenancy::migration::list_index(
            "m0028_tenant_list_index",
            vec!["auth_user", "auth_role"],
        ),
        // Quyền theo nút: độ sâu của permission, membership và API key gắn nút (bước 34, D-304).
        Box::new(m0029_org_scopes::Migration),
        // Ngôn ngữ, múi giờ ưa thích; role bắt buộc MFA (bước 34 mục 9, D-308).
        Box::new(m0030_account::Migration),
        // Danh tính hai lớp: nhân sự và người mua (bước 34 mục 8, D-309).
        Box::new(m0031_realm::Migration),
        // Uỷ quyền theo cây khách hàng (bước 34 mục 10, D-310).
        Box::new(m0032_delegations::Migration),
        // Lời mời người dùng qua email (bước 34 mục 12, D-312).
        Box::new(m0033_invitations::Migration),
        // Quyền vào hỗ trợ của nhân sự nền tảng (bước 34 mục 13, D-313).
        Box::new(m0034_support_grants::Migration),
        // Loại API key (bước 51 P12).
        Box::new(m0035_api_key_kind::Migration),
        // Đường người làm vào khách (bước 34, D-310).
        autoxide_kernel::event::outbox_access_migration("k0001_event_access", "auth"),
        // Đăng nhập qua nhà cung cấp danh tính (bước 40 P3).
        Box::new(m0036_oidc::Migration),
        // Deny thắng allow trên role (bước 54).
        Box::new(m0037_role_denies::Migration),
        // API key: dải IP được gọi, hạn bắt buộc (bước 54).
        Box::new(m0038_api_key_networks::Migration),
    ]
}

/// Upgrade dữ liệu của module, theo thứ tự.
pub(crate) fn upgrades() -> Vec<Box<dyn autoxide_kernel::migration::DataUpgrade>> {
    vec![Box::new(u0001_seal_totp::SealTotp)]
}
