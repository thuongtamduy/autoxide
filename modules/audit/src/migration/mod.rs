//! Migration của module `audit`, theo thứ tự áp dụng. Đã commit thì không sửa —
//! thêm file mới và một dòng vào cuối [`all`].

mod m0001_create_entry;
mod m0002_add_actor_kind;
mod m0003_add_module_index;
mod m0004_add_hash_chain;

use autoxide_kernel::migration::prelude::MigrationTrait;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(m0001_create_entry::Migration),
        Box::new(m0002_add_actor_kind::Migration),
        Box::new(m0003_add_module_index::Migration),
        Box::new(m0004_add_hash_chain::Migration),
    ]
}
