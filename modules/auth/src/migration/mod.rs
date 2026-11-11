//! Migration của module `auth`, theo thứ tự áp dụng. Đã commit thì không sửa —
//! thêm file mới và một dòng vào cuối [`all`].

mod m0001_create_user;
mod m0002_create_refresh_token;
mod m0003_create_roles;
mod m0004_create_login_throttle;

use autoxide_kernel::migration::prelude::MigrationTrait;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(m0001_create_user::Migration),
        Box::new(m0002_create_refresh_token::Migration),
        Box::new(m0003_create_roles::Migration),
        Box::new(m0004_create_login_throttle::Migration),
    ]
}
