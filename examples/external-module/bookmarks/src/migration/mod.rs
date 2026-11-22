//! Migrations of the module, in the order they are applied. A migration that was released is
//! never edited: add a new file and one line at the end of [`all`].

mod m0001_create_bookmark;

use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::migration::{HelperTable, helper_tables};

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    let mut migrations: Vec<Box<dyn MigrationTrait>> =
        vec![Box::new(m0001_create_bookmark::Migration)];
    // Helper tables the kernel defines and this module owns: `bookmarks_events` (the
    // transactional outbox behind `Outbox::record`) and `bookmarks_jobs` (the job queue).
    migrations.extend(helper_tables("bookmarks", &[HelperTable::Events, HelperTable::Jobs]));
    migrations
}
