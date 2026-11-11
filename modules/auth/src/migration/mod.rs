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
    ]
}
