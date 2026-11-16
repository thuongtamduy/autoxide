//! Lệnh `autoxide placement` (bước 33): thêm, liệt kê, kiểm placement — database giữ dữ liệu của
//! khách ở chế độ nhiều khách. Placement chỉ tạo bằng CLI: URL kết nối có mật khẩu, không đi qua
//! API (đặc tả 33). URL lưu mã hoá; lệnh không bao giờ in URL ra.
//!
//! `storage`: đặt / bỏ nơi lưu file riêng của placement (thư mục hay bucket S3), lưu mã hoá; khoá
//! bí mật đọc từ stdin khi tham số là `-`, không bao giờ in ra.

use std::io::BufRead;
use std::process::ExitCode;

use autoxide_kernel::config::DatabaseUrl;
use autoxide_kernel::database::Database;
use autoxide_kernel::storage::{S3Location, StorageLocation};
use autoxide_kernel::tenancy::{
    DatabaseRegistry, NewPlacement, Placement, PlacementCheck, PlacementKind, Placements,
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
    /// List the placements (never their URLs or storage keys).
    List,
    /// Give a placement its own file storage (a directory or an S3 bucket) for the files of its
    /// tenants, or `--clear` it (files go back to the storage of the `files` module settings).
    /// Settings are stored encrypted; access keys are never shown.
    Storage {
        /// Name of the placement (not `main`).
        name: String,
        /// Directory on the server.
        #[arg(long, conflicts_with_all = ["s3_bucket", "clear"])]
        dir: Option<String>,
        /// S3 bucket (it must exist).
        #[arg(long, conflicts_with = "clear")]
        s3_bucket: Option<String>,
        /// Folder inside the bucket.
        #[arg(long, requires = "s3_bucket")]
        s3_prefix: Option<String>,
        /// Region of the bucket.
        #[arg(long, requires = "s3_bucket", default_value = "us-east-1")]
        s3_region: String,
        /// Address of an S3-compatible service; omit for Amazon S3.
        #[arg(long, requires = "s3_bucket")]
        s3_endpoint: Option<String>,
        /// Access key; omit on AWS to use the server's IAM role.
        #[arg(long, requires = "s3_bucket")]
        s3_access_key_id: Option<String>,
        /// Secret of the access key; `-` reads it from standard input, which keeps it out of the
        /// shell history and the process list.
        #[arg(long, requires = "s3_access_key_id")]
        s3_secret_access_key: Option<String>,
        /// Remove the placement's own storage.
        #[arg(long)]
        clear: bool,
        /// Change it although tenants keep data on the placement — only when the new storage
        /// already holds the same files (e.g. a new access key for the same bucket).
        #[arg(long)]
        force: bool,
    },
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
                .iter()
                .map(placement_line)
                .collect();
            Ok(lines.join("\n"))
        }
        Action::Storage {
            name,
            dir,
            s3_bucket,
            s3_prefix,
            s3_region,
            s3_endpoint,
            s3_access_key_id,
            s3_secret_access_key,
            clear,
            force,
        } => {
            let location = match (dir, s3_bucket, clear) {
                (_, _, true) => None,
                (Some(dir), _, false) => Some(StorageLocation::Local { dir }),
                (None, Some(bucket), false) => {
                    let secret = match s3_secret_access_key {
                        Some(raw) if raw == "-" => {
                            Some(read_line(&mut std::io::stdin().lock(), "the secret")?)
                        }
                        other => other,
                    };
                    Some(StorageLocation::S3(S3Location {
                        bucket,
                        prefix: s3_prefix,
                        region: s3_region,
                        endpoint: s3_endpoint,
                        access_key_id: s3_access_key_id,
                        secret_access_key: secret,
                    }))
                }
                (None, None, false) => {
                    return Err("give --dir, --s3-bucket or --clear".to_owned());
                }
            };
            let placement = placements
                .set_storage(&name, location, force)
                .await
                .map_err(|error| describe_error(&error))?;
            Ok(placement_line(&placement))
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

/// Một dòng của `input` (bỏ khoảng trắng hai đầu); `what` cho thông báo lỗi.
fn read_line(input: &mut impl BufRead, what: &str) -> Result<String, String> {
    let mut line = String::new();
    input.read_line(&mut line).map_err(|error| format!("cannot read {what}: {error}"))?;
    Ok(line.trim().to_owned())
}

/// Một dòng mô tả placement (không URL, không khoá bí mật).
fn placement_line(placement: &Placement) -> String {
    let storage = match &placement.storage {
        None => "storage=shared".to_owned(),
        Some(storage) => match (storage.dir.as_deref(), storage.bucket.as_deref()) {
            (Some(dir), _) => format!("storage=local:{dir}"),
            (None, Some(bucket)) => format!(
                "storage=s3:{bucket}{}",
                storage.prefix.as_deref().map(|prefix| format!("/{prefix}")).unwrap_or_default()
            ),
            (None, None) => format!("storage={}", storage.kind),
        },
    };
    format!(
        "{}  {}  {}  {}  {}  {storage}",
        placement.id, placement.name, placement.kind, placement.backend, placement.schema_state
    )
}

/// URL từ tham số, hay một dòng của `input` khi tham số là `-`.
fn parse_url(raw: &str, input: &mut impl BufRead) -> Result<DatabaseUrl, String> {
    let raw = if raw == "-" { read_line(input, "the URL")? } else { raw.trim().to_owned() };
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
    use autoxide_kernel::config::ConfigSource;
    use autoxide_kernel::tenancy::{TenancyConfig, TenancyMode};
    use autoxide_testkit::TestDatabase;

    /// Dòng lệnh như người vận hành gõ (`autoxide placement …`).
    fn parse(line: &[&str]) -> Result<Action, String> {
        #[derive(clap::Parser)]
        struct Cli {
            #[command(flatten)]
            arguments: Arguments,
        }
        let args = std::iter::once("placement").chain(line.iter().copied());
        <Cli as clap::Parser>::try_parse_from(args)
            .map(|cli| cli.arguments.action)
            .map_err(|error| error.to_string())
    }

    /// `storage`: đặt nơi lưu S3 (in bucket, không in khoá), liệt kê, bỏ; thiếu tham số thì báo.
    #[tokio::test]
    async fn dat_va_bo_noi_luu_rieng_khong_in_khoa() {
        TestDatabase::run(async |test| {
            let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
            let settings = crate::settings::read(&source).unwrap();
            let database = Database::connect(&settings.config.database).await.unwrap();
            let registry = DatabaseRegistry::new(
                database.clone(),
                settings.config.database.clone(),
                TenancyConfig::with_mode(TenancyMode::Multi),
            );
            let placements = settings.modules.placements(&registry).unwrap();
            placements.up_all().await.unwrap();
            let url = test.extra_database().await.unwrap();
            let new = NewPlacement::new("eu-1", PlacementKind::Dedicated, url.parse().unwrap());
            placements.add(new).await.unwrap();

            let set = parse(&[
                "storage",
                "eu-1",
                "--s3-bucket",
                "eu-files",
                "--s3-prefix",
                "acme",
                "--s3-access-key-id",
                "KEY-ID-42",
                "--s3-secret-access-key",
                "TOPSECRET-42",
            ])
            .unwrap();
            let line = execute(&placements, set).await.unwrap();
            assert!(line.contains("eu-1") && line.ends_with("storage=s3:eu-files/acme"), "{line}");
            let listed = execute(&placements, parse(&["list"]).unwrap()).await.unwrap();
            assert!(listed.contains("storage=s3:eu-files/acme"), "{listed}");
            assert!(listed.contains("storage=shared"), "{listed}");
            assert!(!listed.contains("TOPSECRET") && !listed.contains("KEY-ID-42"), "{listed}");

            let cleared = execute(&placements, parse(&["storage", "eu-1", "--clear"]).unwrap());
            assert!(cleared.await.unwrap().ends_with("storage=shared"));
            let missing = execute(&placements, parse(&["storage", "eu-1"]).unwrap()).await;
            assert_eq!(missing.unwrap_err(), "give --dir, --s3-bucket or --clear");
            let main = execute(&placements, parse(&["storage", "main", "--dir", "x"]).unwrap());
            assert!(main.await.unwrap_err().contains("placement"));
            // Khoá bí mật đi kèm khoá truy cập; `--dir` và `--s3-bucket` không đi cùng nhau.
            assert!(
                parse(&["storage", "eu-1", "--s3-bucket", "b", "--s3-secret-access-key", "x"])
                    .is_err()
            );
            assert!(parse(&["storage", "eu-1", "--dir", "d", "--s3-bucket", "b"]).is_err());
            database.close().await.unwrap();
        })
        .await
        .unwrap();
    }

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
