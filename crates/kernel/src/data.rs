//! Database access: read / write connections, migrations, pagination, row locks, references, versions
//! (ETag), accent-insensitive search, data transfer.
//!
//! Step 33+ adds `tenancy` here; step 36 adds `numbering` (gap-free sequential numbering).

pub mod backup;
pub mod database;
pub mod migration;
pub mod numbering;
pub mod pagination;
pub mod reference;
pub mod rows;
pub mod search;
pub mod sync;
pub mod tenancy;
pub mod transfer;
pub mod upload;
pub mod version;
