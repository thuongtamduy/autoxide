//! Lệnh `autoxide placement` (bước 33): thêm, liệt kê, kiểm placement — database giữ dữ liệu của
//! khách ở chế độ nhiều khách. Placement chỉ tạo bằng CLI: URL kết nối có mật khẩu, không đi qua
//! API (đặc tả 33). URL lưu mã hoá; lệnh không bao giờ in URL ra.

use std::io::BufRead;
use std::process::ExitCode;

use autoxide_kernel::config::DatabaseUrl;
use autoxide_kernel::database::Database;
use autoxide_kernel::tenancy::{
    DatabaseRegistry, NewPlacement, PlacementCheck, PlacementKind, Placements,
};

use super::{
    EXIT_FAILURE, describe_error, report_failure, report_invalid_configuration,
    run_with_observability,
};
use crate::settings;

#[derive(Debug, clap::Args)]
pub struct Arguments {
    #[command(subcommand)]
    action: Action,
}

#[derive(Debug, Clone, clap::Subcommand)]
enum Action {
    /// Add a placement. The URL is checked by connecting, then stored encrypted.
    Add {
        /// Name: lower-case letters, digits and `-` (not `main`).
        name: String,
        /// Connection URL (`postgres://…` or `sqlite://…?mode=rwc`); `-` reads it from standard
        /// input, which keeps the password out of the shell history and the process list.
        #[arg(long)]
        url: String,
        /// Read replica URL for reads (PostgreSQL); `-` reads it from standard input.
        #[arg(long)]
        read_url: Option<String>,
        /// `shared` (many tenants) or `dedicated` (one tenant).
        #[arg(long, default_value = "shared")]
        kind: String,
    },
    /// List the placements (never their URLs).
    List,
    /// Compare every placement's migrations with the program and record the result.
    Check,
}

pub fn run(arguments: &Arguments) -> ExitCode {
    let settings = match settings::load() {
        Ok(settings) => settings,
        Err(errors) => return report_invalid_configuration(&errors),
    };
    let config = settings.config.clone();
    let action = arguments.action.clone();
    let outcome = run_with_observability(&config, async move {
        let database = Database::connect(&settings.config.database)
            .await
            .map_err(|error| error.to_string())?;
        let outcome = match crate::runtime::require_applied(&settings.modules, &database).await {
            Ok(()) => {
                let registry = DatabaseRegistry::new(
                    database.clone(),
                    settings.config.database.clone(),
                    settings.config.tenancy.clone(),
                );
                match settings.modules.placements(&registry) {
                    Ok(placements) => execute(&placements, action).await,
                    Err(error) => Err(error.to_string()),
                }
            }
            Err(error) => Err(error),
        };
        database.close().await.map_err(|error| format!("database: {error}"))?;
        Ok(outcome)
    });
    match outcome {
        Ok(Ok(report)) => {
            println!("{report}");
            ExitCode::SUCCESS
        }
        Ok(Err(message)) => {
            eprintln!("autoxide placement: {message}");
            ExitCode::from(EXIT_FAILURE)
        }
        Err(error) => report_failure(&error),
    }
}

async fn execute(placements: &Placements, action: Action) -> Result<String, String> {
    match action {
        Action::Add { name, url, read_url, kind } => {
            let kind = PlacementKind::parse(&kind)
                .ok_or_else(|| "--kind must be `shared` or `dedicated`".to_owned())?;
            let mut stdin = std::io::stdin().lock();
            let mut new = NewPlacement::new(name, kind, parse_url(&url, &mut stdin)?);
            if let Some(read_url) = read_url {
                new = new.read_url(parse_url(&read_url, &mut stdin)?);
            }
            Ok(check_line(&placements.add(new).await.map_err(|error| describe_error(&error))?))
        }
        Action::List => {
            let lines: Vec<String> = placements
                .list()
                .await
                .map_err(|error| describe_error(&error))?
                .into_iter()
                .map(|placement| {
                    format!(
                        "{}  {}  {}  {}  {}",
                        placement.id,
                        placement.name,
                        placement.kind,
                        placement.backend,
                        placement.schema_state
                    )
                })
                .collect();
            Ok(lines.join("\n"))
        }
        Action::Check => {
            let checks = placements.check().await.map_err(|error| describe_error(&error))?;
            if checks.is_empty() {
                return Ok("No placements besides `main`.".to_owned());
            }
            Ok(checks.iter().map(check_line).collect::<Vec<_>>().join("\n"))
        }
    }
}

/// URL từ tham số, hay một dòng của `input` khi tham số là `-`.
fn parse_url(raw: &str, input: &mut impl BufRead) -> Result<DatabaseUrl, String> {
    let raw = if raw == "-" {
        let mut line = String::new();
        input.read_line(&mut line).map_err(|error| format!("cannot read the URL: {error}"))?;
        line.trim().to_owned()
    } else {
        raw.trim().to_owned()
    };
    // Thông báo lỗi của `DatabaseUrl` không chứa URL (không lộ mật khẩu).
    raw.parse::<DatabaseUrl>().map_err(|error| format!("the URL {error}"))
}

/// Một dòng mô tả kết quả kiểm placement.
fn check_line(check: &PlacementCheck) -> String {
    let mut line = format!("{}  {}  {}", check.id, check.name, check.state.as_str());
    if check.pending > 0 {
        line.push_str(&format!("  {} migrations pending", check.pending));
    }
    if let Some(error) = &check.error {
        line.push_str(&format!("  error: {error}"));
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_url_tu_stdin_khi_la_dau_gach() {
        let mut input = std::io::Cursor::new("sqlite://data/p1.sqlite?mode=rwc\n");
        let url = parse_url("-", &mut input).unwrap();
        assert_eq!(url.expose(), "sqlite://data/p1.sqlite?mode=rwc");

        let error = parse_url("mysql://x", &mut std::io::Cursor::new("")).unwrap_err();
        assert!(error.starts_with("the URL must start with"), "{error}");
        assert!(!error.contains("mysql://x"), "{error}");
    }
}
