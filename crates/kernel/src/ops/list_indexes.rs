//! Indexes for the paged "mine" lists (step 70 P9 item 3, D-20261125-list-pages): every list paged with a
//! keyset has an index on its filter columns + sort column + ID, so the database jumps straight to where a
//! page starts. `GET /me/flags` needs none: it pages by key on the unique index `(tenant_id, user_id, key)`.
//! No older index is a prefix of a new one, so none is dropped.

use crate::migration::prelude::*;

pub(crate) struct AddListIndexes;

impl MigrationName for AddListIndexes {
    fn name(&self) -> &str {
        "m0041_list_page_indexes"
    }
}

/// `(table, index name, columns)` added.
const ADDED: [(&str, &str, &[&str]); 3] = [
    // `GET /me/pins`: the caller's pins by position.
    ("kernel_pins", "kernel_pins_page_idx", &["tenant_id", "user_id", "position", "id"]),
    // `GET /me/views`: newest first, with or without `resource`.
    ("kernel_saved_views", "kernel_saved_views_page_idx", &["tenant_id", "id"]),
    (
        "kernel_saved_views",
        "kernel_saved_views_resource_page_idx",
        &["tenant_id", "resource", "id"],
    ),
];

#[async_trait::async_trait]
impl MigrationTrait for AddListIndexes {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for (table, name, columns) in ADDED {
            let mut index = Index::create();
            index.name(name).table(Alias::new(table));
            for column in columns {
                index.col(Alias::new(*column));
            }
            manager.create_index(index.to_owned()).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for (table, name, _) in ADDED {
            manager
                .drop_index(Index::drop().name(name).table(Alias::new(table)).to_owned())
                .await?;
        }
        Ok(())
    }
}

/// Indexes for the kernel lists paged in cluster 3 (D-20261125-list-pages): `GET /jobs` without a filter or by
/// `kind` (by `requested_by` and by `status` already have theirs), and `GET /custom-fields` by `position` with
/// or without `target`. No older index is a prefix of a new one, so none is dropped.
pub(crate) struct AddListIndexesKernel;

impl MigrationName for AddListIndexesKernel {
    fn name(&self) -> &str {
        "m0042_list_page_indexes_kernel"
    }
}

/// `(table, index name, columns)` added.
const KERNEL_ADDED: [(&str, &str, &[&str]); 4] = [
    ("kernel_job_runs", "kernel_job_runs_page_idx", &["tenant_id", "id"]),
    ("kernel_job_runs", "kernel_job_runs_kind_page_idx", &["tenant_id", "kind", "id"]),
    ("kernel_custom_fields", "kernel_custom_fields_page_idx", &["tenant_id", "position", "id"]),
    (
        "kernel_custom_fields",
        "kernel_custom_fields_target_page_idx",
        &["tenant_id", "target", "position", "id"],
    ),
];

#[async_trait::async_trait]
impl MigrationTrait for AddListIndexesKernel {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for (table, name, columns) in KERNEL_ADDED {
            let mut index = Index::create();
            index.name(name).table(Alias::new(table));
            for column in columns {
                index.col(Alias::new(*column));
            }
            manager.create_index(index.to_owned()).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for (table, name, _) in KERNEL_ADDED {
            manager
                .drop_index(Index::drop().name(name).table(Alias::new(table)).to_owned())
                .await?;
        }
        Ok(())
    }
}
