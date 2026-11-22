//! Migrations of the module, in the order they are applied. A migration that was released is
//! never edited: add a new file and one line at the end of [`all`].

mod m0001_create_bookmark;

use autoxide_kernel::migration::prelude::MigrationTrait;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(m0001_create_bookmark::Migration),
        // Helper tables the kernel defines and this module owns: `bookmarks_events` (the
        // transactional outbox behind `Outbox::record`) and `bookmarks_jobs` (the job queue).
        autoxide_kernel::event::outbox_migration("m0002_create_events", "bookmarks"),
        autoxide_kernel::event::outbox_actor_migration("m0003_add_event_actor", "bookmarks"),
        autoxide_kernel::job::jobs_migration("m0004_create_jobs", "bookmarks"),
        // Gives the helper tables their `tenant_id`.
        autoxide_kernel::tenancy::migration::adopt(
            "m0005_tenancy_helpers",
            vec![
                autoxide_kernel::event::tenant_table("bookmarks"),
                autoxide_kernel::job::tenant_table("bookmarks"),
            ],
        ),
        autoxide_kernel::event::outbox_access_migration("m0006_event_access", "bookmarks"),
    ]
}
