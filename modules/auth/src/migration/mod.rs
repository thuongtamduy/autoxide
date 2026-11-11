//! Migration của module `auth`, theo thứ tự áp dụng. Đã commit thì không sửa —
//! thêm file mới và một dòng vào cuối [`all`].

mod m0001_create_user;
mod m0002_create_refresh_token;

use autoxide_kernel::migration::prelude::MigrationTrait;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![Box::new(m0001_create_user::Migration), Box::new(m0002_create_refresh_token::Migration)]
}
