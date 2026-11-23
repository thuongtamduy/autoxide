//! Contract of the `notes` module for other modules: only what is here may be used from
//! outside (`autoxide_notes::api::…`). Other modules obtain the trait at startup:
//!
//! ```text
//! let notes = context.require::<dyn autoxide_notes::api::Notes>()?;
//! ```
//!
//! That module must be registered AFTER `notes` in `crates/app/src/modules.rs`.

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::TenantContext;
use uuid::Uuid;

/// Summary of a note for other modules (without its content).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteSummary {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub title: String,
}

/// Looks up notes, e.g. so another module can check that a note ID really exists (no cross-module
/// foreign key — architecture rule 4).
//
// `#[async_trait]`: Rust chưa cho gọi hàm `async` của trait qua `dyn Trait`; macro này
// đổi hàm `async` thành hàm trả về `Box<dyn Future>` để gọi được.
#[async_trait::async_trait]
pub trait Notes: Send + Sync {
    /// The note with ID `id` of tenant `tenant`, or `None` if there is none.
    async fn find(&self, tenant: &TenantContext, id: Uuid) -> Result<Option<NoteSummary>, Error>;
}
