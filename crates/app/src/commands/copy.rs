//! Lệnh `autoxide copy`: chép dữ liệu của mọi module từ database của máy chủ này
//! (`DATABASE_URL`) sang một database khác, trống — SQLite sang PostgreSQL hay ngược
//! lại (D-070).

use std::io::BufRead;
use std::process::ExitCode;

use autoxide_kernel::config::{ConfigReader, ConfigSource, DatabaseConfig};
use autoxide_kernel::database::Database;
use autoxide_kernel::transfer::copy_modules;

use super::{report_failure, report_invalid_configuration, run_with_observability};
use crate::settings::{self, Settings};

#[derive(Debug, Clone, clap::Args)]
#[group(required = true, multiple = false)]
pub struct Arguments {
    /// URL of the target database, like `DATABASE_URL`. Visible to other users of
    /// this machine in the process list: prefer `--to-stdin` when it has a password.
    #[arg(long, value_name = "URL")]
    to: Option<String>,
    /// Read the target URL from the first line of standard input:
    /// `printf '%s' "$TARGET_URL" | autoxide copy --to-stdin`.
    #[arg(long)]
    to_stdin: bool,
}

pub fn run(arguments: &Arguments) -> ExitCode {
    let settings = match settings::load() {
        Ok(settings) => settings,
        Err(errors) => return report_invalid_configuration(&errors),
    };
    let url = match (&arguments.to, arguments.to_stdin) {
        (Some(url), _) => url.trim().to_owned(),
        (None, _) => {
            let mut line = String::new();
            if let Err(error) = std::io::stdin().lock().read_line(&mut line) {
                return report_failure(&format!("cannot read the target URL: {error}"));
            }
            line.trim().to_owned()
        }
    };
    // Kiểm URL đích như `DATABASE_URL`, trước khi kết nối gì.
    let source = ConfigSource::from_pairs([("DATABASE_URL", url.as_str())]);
    let mut reader = ConfigReader::new(&source);
    let target = DatabaseConfig::read(&mut reader);
    let target = match reader.finish(target) {
        Ok((target, _)) => target,
        Err(errors) => return report_invalid_configuration(&errors),
    };
    let config = settings.config.clone();
    match run_with_observability(&config, copy(settings, target)) {
        Ok(report) => {
            print!("{report}");
            ExitCode::SUCCESS
        }
        Err(error) => report_failure(&error),
    }
}

async fn copy(settings: Settings, target: DatabaseConfig) -> Result<String, String> {
    let migrator = settings.modules.migrator().map_err(|error| error.to_string())?;
    let source = Database::connect(&settings.config.database).await.map_err(|e| e.to_string())?;
    let pending = migrator.pending(&source).await.map_err(|error| error.to_string())?;
    if !pending.is_empty() {
        let _ = source.close().await;
        return Err(format!(
            "the source database has {} migration(s) not applied; run `autoxide migrate up` \
             first so both databases get the same schema",
            pending.len()
        ));
    }
    // Chỉ bảng của module đang bật được chép: còn dữ liệu của module đang tắt thì dừng, không để
    // nó ở lại trong im lặng (bước 70 P1).
    if let Err(error) =
        crate::modules::refuse_dormant_data(&settings.modules, &source, "copy").await
    {
        let _ = source.close().await;
        return Err(error);
    }
    let target = match Database::connect(&target).await {
        Ok(target) => target,
        Err(error) => {
            let _ = source.close().await;
            return Err(format!("target database: {error}"));
        }
    };
    let result = async {
        // Chỉ chép vào database mới: dữ liệu module ở đích sẽ bị thay.
        let applied = migrator.status(&target).await.map_err(|error| error.to_string())?;
        if applied.iter().any(|state| state.applied) {
            return Err("the target database already has Autoxide tables; copy into a new, \
                        empty database"
                .to_owned());
        }
        // Đích nhận cùng schema với nguồn: mọi migration của mọi module.
        migrator.up(&target).await.map_err(|error| error.to_string())?;
        let names = settings.modules.names();
        copy_modules(&source, &target, &names).await.map_err(|error| error.to_string())
    }
    .await;
    let _ = source.close().await;
    let _ = target.close().await;
    let copied = result?;
    let total: u64 = copied.iter().map(|table| table.rows).sum();
    let mut report = String::new();
    for table in &copied {
        report.push_str(&format!("{:>10}  {}\n", table.rows, table.table));
    }
    report.push_str(&format!("Copied {total} rows in {} tables.\n", copied.len()));
    Ok(report)
}
