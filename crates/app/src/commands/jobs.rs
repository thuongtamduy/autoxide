//! Lệnh `autoxide jobs`: người vận hành xem job hỏng hẳn (dead-letter queue) và chạy
//! lại sau khi sửa xong nguyên nhân.

use std::process::ExitCode;

use autoxide_kernel::job::{JobRecord, JobStatus};
use uuid::Uuid;

use super::{report_failure, report_invalid_configuration, run_with_observability};
use crate::runtime::{self, Migrations};
use crate::settings::{self, Settings};

#[derive(Debug, clap::Args)]
pub struct Arguments {
    #[command(subcommand)]
    action: Action,
}

#[derive(Debug, Clone, clap::Subcommand)]
pub enum Action {
    /// List jobs that failed for good (the dead-letter queue), newest first.
    Failed {
        /// Only this module's jobs.
        #[arg(long)]
        module: Option<String>,
        /// How many jobs to show per module.
        #[arg(long, default_value_t = 50)]
        limit: u64,
    },
    /// Queue a failed job again, as new. Fix what made it fail first.
    Retry {
        /// Module that owns the job, as shown by `autoxide jobs failed`.
        module: String,
        /// Id of the job.
        id: Uuid,
    },
}

pub fn run(arguments: &Arguments) -> ExitCode {
    let settings = match settings::load() {
        Ok(settings) => settings,
        Err(errors) => return report_invalid_configuration(&errors),
    };
    let config = settings.config.clone();
    match run_with_observability(&config, execute(settings, arguments.action.clone())) {
        Ok(report) => {
            print!("{report}");
            ExitCode::SUCCESS
        }
        Err(error) => report_failure(&error),
    }
}

async fn execute(settings: Settings, action: Action) -> Result<String, String> {
    let runtime = runtime::start(settings, Migrations::RequireApplied).await?;
    let worker = &runtime.worker;
    let outcome = async {
        match action {
            Action::Failed { module, limit } => {
                let modules: Vec<&str> = match &module {
                    Some(module) => vec![module.as_str()],
                    None => worker.modules().to_vec(),
                };
                let mut report = String::new();
                for module in modules {
                    let jobs = worker
                        .list(module, JobStatus::Failed, limit)
                        .await
                        .map_err(|error| error.message())?;
                    report.push_str(&render(module, &jobs));
                }
                if report.is_empty() {
                    report.push_str("No failed jobs.\n");
                }
                Ok(report)
            }
            Action::Retry { module, id } => {
                worker.retry(&module, id).await.map_err(|error| error.message())?;
                Ok(format!("Job {id} of module `{module}` is queued again.\n"))
            }
        }
    }
    .await;
    let outcome: Result<String, String> = outcome;
    // Đóng kết nối cả khi lệnh hỏng; lỗi của lệnh quan trọng hơn lỗi lúc đóng.
    let closed = runtime.database.close().await.map_err(|error| format!("database: {error}"));
    let report = outcome?;
    closed?;
    Ok(report)
}

/// Các job hỏng của một module, mỗi job một khối.
fn render(module: &str, jobs: &[JobRecord]) -> String {
    let mut output = String::new();
    for job in jobs {
        let failed_at = job.failed_at.map(|at| at.to_rfc3339()).unwrap_or_default();
        output.push_str(&format!(
            "{module} {} {} — {} of {} attempts, failed at {failed_at}\n    {}\n",
            job.id,
            job.kind,
            job.attempts,
            job.max_attempts,
            job.last_error.as_deref().unwrap_or("(no error recorded)")
        ));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};

    #[test]
    fn in_job_hong() {
        let job = JobRecord {
            id: Uuid::nil(),
            kind: "reports.send".to_owned(),
            attempts: 10,
            max_attempts: 10,
            run_at: DateTime::<Utc>::UNIX_EPOCH,
            failed_at: Some(DateTime::<Utc>::UNIX_EPOCH),
            last_error: Some("internal_error: the mail server is down".to_owned()),
        };

        assert_eq!(
            render("reports", &[job]),
            "reports 00000000-0000-0000-0000-000000000000 reports.send — 10 of 10 attempts, \
             failed at 1970-01-01T00:00:00+00:00\n    internal_error: the mail server is down\n"
        );
    }
}
