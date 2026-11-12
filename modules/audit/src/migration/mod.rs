//! Migration của module `audit`, theo thứ tự áp dụng. Đã commit thì không sửa —
//! thêm file mới và một dòng vào cuối [`all`].

mod m0001_create_entry;
mod m0002_add_actor_kind;

use autoxide_kernel::migration::prelude::MigrationTrait;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![Box::new(m0001_create_entry::Migration), Box::new(m0002_add_actor_kind::Migration)]
}
