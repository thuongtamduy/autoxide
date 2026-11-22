//! The daily job that deletes bookmarks older than `ACME_BOOKMARKS_RETENTION_DAYS`.
//!
//! A schedule puts one job a day into the queue of every database; the worker of `acme serve`
//! (or of a process with `AUTOXIDE_PROCESS_ROLE=worker`) runs it. A job that fails is retried
//! with backoff, so it must be safe to run again.

use std::sync::Arc;
use std::time::Duration;

use autoxide_kernel::error::Error;
use autoxide_kernel::job::{JobContext, JobHandler, Schedule};
use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use crate::entity::{Column, Entity};
use crate::service::BookmarkService;

/// The job kind, also the name of the schedule: `<module>.<work>`.
pub(crate) const CLEANUP: &str = "bookmarks.cleanup";

pub(crate) fn schedule() -> Schedule {
    Schedule::new(CLEANUP, CLEANUP, Duration::from_secs(24 * 3600))
}

pub(crate) struct Cleanup {
    service: Arc<BookmarkService>,
}

impl Cleanup {
    pub(crate) fn new(service: Arc<BookmarkService>) -> Self {
        Cleanup { service }
    }
}

#[async_trait::async_trait]
impl JobHandler for Cleanup {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let days = self.service.config().retention_days;
        if days == 0 {
            return Ok(());
        }
        let cutoff = Timestamp::now() - SignedDuration::from_hours(24 * i64::from(days));
        // tenant-scope: the retention job of a database deletes the old bookmarks of every tenant
        // in it; the setting is one for the whole installation.
        let removed = Entity::delete_many()
            .filter(Column::CreatedAt.lt(cutoff))
            .exec(job.database.write())
            .await?;
        tracing::info!(removed = removed.rows_affected, "old bookmarks removed");
        Ok(())
    }
}
