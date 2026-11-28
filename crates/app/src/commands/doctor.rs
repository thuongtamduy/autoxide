//! Lệnh `autoxide doctor`: kiểm bản cài đặt từng bước và nói cách sửa (D-166).
//!
//! Thứ tự: cấu hình → database (đọc và ghi) → migration → khoá ký → từng bước kiểm module khai
//! (`Module::checks`). Bước nền hỏng thì các bước dựa vào nó không chạy (báo `skipped`), để
//! người đọc thấy đúng một nguyên nhân thay vì một loạt lỗi kéo theo.

use std::process::ExitCode;

use autoxide_kernel::database::Database;
use autoxide_kernel::doctor::{Finding, Status};
use autoxide_kernel::module::Modules;
use autoxide_kernel::secret::SecretKey;
use serde::Serialize;

use super::{EXIT_FAILURE, EXIT_INVALID_CONFIGURATION};
use crate::settings::{self, Settings};

#[derive(Debug, clap::Args)]
pub struct Arguments {
    /// Print the report as JSON (for scripts and AI agents) instead of text.
    #[arg(long)]
    json: bool,
}

/// Một dòng của báo cáo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct Line {
    name: String,
    /// `ok`, `warning`, `failed`, hoặc `skipped` (không chạy vì bước nền trước đó hỏng).
    status: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<String>,
}

#[derive(Debug, Serialize)]
struct Report {
    healthy: bool,
    checks: Vec<Line>,
}

pub fn run(arguments: &Arguments) -> ExitCode {
    let (report, configuration_valid) = match settings::load() {
        Ok(settings) => match super::block_on(async { Ok(diagnose(settings).await) }) {
            Ok(lines) => (lines, true),
            Err(error) => return super::report_failure(&error),
        },
        Err(errors) => {
            let finding = Finding::failed(errors.to_string()).with_hint(
                "Settings come from environment variables and `.env`; see `.env.example` for \
                 every setting and its default.",
            );
            (vec![line("configuration", Some(finding))], false)
        }
    };
    let healthy = report.iter().all(|line| matches!(line.status, "ok" | "warning"));
    if arguments.json {
        let report = Report { healthy, checks: report };
        match serde_json::to_string_pretty(&report) {
            Ok(json) => println!("{json}"),
            Err(error) => return super::report_failure(&error.to_string()),
        }
    } else {
        print!("{}", render(&report, healthy));
    }
    if !configuration_valid {
        ExitCode::from(EXIT_INVALID_CONFIGURATION)
    } else if healthy {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(EXIT_FAILURE)
    }
}

/// Cảnh báo vẫn tính là đạt: chạy được, chỉ nên sửa.
fn passes(finding: &Finding) -> bool {
    finding.status != Status::Failed
}

/// `None`: bước không chạy vì bước nền trước đó hỏng.
fn line(name: &str, finding: Option<Finding>) -> Line {
    match finding {
        Some(finding) => Line {
            name: name.to_owned(),
            status: match finding.status {
                Status::Ok => "ok",
                Status::Warning => "warning",
                Status::Failed => "failed",
            },
            message: finding.message,
            hint: finding.hint,
        },
        None => Line {
            name: name.to_owned(),
            status: "skipped",
            message: "Not run: an earlier check failed.".to_owned(),
            hint: None,
        },
    }
}

/// Chạy mọi bước sau khi cấu hình đã hợp lệ.
async fn diagnose(settings: Settings) -> Vec<Line> {
    let Settings { config, mut modules, values, .. } = settings;
    let mut lines = vec![line("configuration", Some(Finding::ok("The configuration is valid.")))];
    lines.push(line("modules", Some(Finding::ok(crate::modules::summary(&modules)))));
    if let Some(finding) = probe_hsts(&values, &config.http) {
        lines.push(line("hsts", Some(finding)));
    }
    if let Some(finding) = probe_rate_limit_store(&config) {
        lines.push(line("rate_limit_store", Some(finding)));
    }
    let database = match Database::connect(&config.database).await {
        Ok(database) => database,
        Err(error) => {
            lines.push(line(
                "database",
                Some(Finding::failed(error.to_string()).with_hint(
                    "Check DATABASE_URL, that the database server is running and that this \
                     server can reach it.",
                )),
            ));
            lines.extend(skipped(["migrations", "secret_key"]));
            return lines;
        }
    };
    let database_ok = probe_database(&database).await;
    let usable = passes(&database_ok);
    lines.push(line("database", Some(database_ok)));
    let migrations = if usable { Some(probe_migrations(&modules, &database).await) } else { None };
    let migrated = migrations.as_ref().is_some_and(passes);
    lines.push(line("migrations", migrations));
    // Module chạy trên bảng chưa có thì mọi bước sau chỉ báo lỗi kéo theo.
    // Record của module khớp schema (D-20261126-kernel-data-layer) rồi mới khởi động; lỗi nào cũng báo ở `modules`.
    let started = if migrated {
        match modules.check_records(&database).await {
            Ok(()) => modules.start(&database),
            Err(error) => Err(error),
        }
    } else {
        Ok(())
    };
    if migrated && let Err(error) = started {
        lines.push(line(
            "modules",
            Some(Finding::failed(error.to_string()).with_hint(
                "A module could not start, or a record does not match its table; the message names the module and \
                 what is missing.",
            )),
        ));
        lines.extend(skipped(["secret_key"]));
    } else if migrated {
        lines.push(line("secret_key", Some(probe_secret_key(&modules))));
        for (module, check) in modules.checks() {
            let name = format!("{module}.{}", check.name);
            lines.push(line(&name, Some(check.run().await)));
        }
    } else {
        lines.extend(skipped(["secret_key"]));
    }
    // Sau khi (thử) khởi động module: lúc đó xem được cả database của từng placement.
    if usable && let Some(finding) = probe_dormant(&modules, &database).await {
        lines.push(line("modules_turned_off", Some(finding)));
    }
    let _ = database.close().await;
    lines
}

/// Các bước không chạy được vì bước nền hỏng. Bước kiểm của module chỉ có sau `start`, nên ở
/// đây chỉ liệt kê các bước của chính lệnh.
fn skipped<const N: usize>(names: [&str; N]) -> Vec<Line> {
    names.iter().map(|name| line(name, None)).collect()
}

/// Hỏi cả kết nối ghi và kết nối đọc (hai pool, có thể hai máy chủ).
async fn probe_database(database: &Database) -> Finding {
    match database.ping().await {
        Ok(()) => {
            Finding::ok(format!("Connected to {:?} for reading and writing.", database.backend()))
        }
        Err(error) => Finding::failed(format!("the database does not answer: {error}"))
            .with_hint("Check DATABASE_URL (and DATABASE_READ_URL if set)."),
    }
}

async fn probe_migrations(modules: &Modules, database: &Database) -> Finding {
    let pending = match modules.migrator() {
        Ok(migrator) => match migrator.pending(database).await {
            Ok(pending) => Ok(pending),
            Err(error) => match error.database_newer() {
                // Database mới hơn binary (bước 71 G3): không phải migration hỏng, cách sửa khác hẳn.
                Some(newer) => {
                    return Finding::failed(newer.error().message())
                        .with_hint(newer_database_hint(newer.module()));
                }
                None => Err(error.to_string()),
            },
        },
        Err(error) => Err(error.to_string()),
    };
    match pending {
        Ok(pending) if pending.is_empty() => Finding::ok("Every migration has been applied."),
        Ok(pending) => {
            let names: Vec<String> =
                pending.iter().map(|state| format!("{}/{}", state.module, state.name)).collect();
            // Bản cài mới có hàng chục migration: nêu vài cái đầu là đủ biết chuyện gì.
            const SHOWN: usize = 5;
            let mut listed = names.iter().take(SHOWN).cloned().collect::<Vec<_>>().join(", ");
            if names.len() > SHOWN {
                listed.push_str(&format!(" and {} more", names.len() - SHOWN));
            }
            Finding::failed(format!("{} migration(s) have not been applied: {listed}", names.len()))
                .with_hint("Run `autoxide migrate up`.")
        }
        Err(error) => Finding::failed(error),
    }
}

/// Cách sửa khi database mới hơn binary. `rollback --to` chỉ lùi module có trong bảng phiên bản nhúng (`releases.rs`):
/// module ghép từ ngoài (`.module(…)`) bị nó để nguyên, nên khi đó chỉ khuyên chạy bản mới hay khôi phục backup.
fn newer_database_hint(module: &str) -> String {
    let version = env!("CARGO_PKG_VERSION");
    let tracked = crate::releases::MigrationReleases::embedded()
        .is_ok_and(|releases| releases.tracks_module(module));
    if tracked {
        format!(
            "Run the newer Autoxide that migrated this database, or go back with `autoxide rollback --to \
             {version}` run by that newer binary."
        )
    } else {
        format!(
            "Run the newer binary that migrated this database, or restore the backup taken before the upgrade: \
             module `{module}` is not in the release table of Autoxide {version}, so `autoxide rollback --to` \
             leaves it as it is."
        )
    }
}

/// Module đang tắt mà database còn dữ liệu của nó (bước 70 P3): người nâng cấp từ bản mà mọi
/// module đều chạy phải thấy điều này. Tự đặt `AUTOXIDE_MODULES` thì chỉ là thông tin; đang theo
/// mặc định thì là cảnh báo. Không có module nào như vậy thì không có bước này.
async fn probe_dormant(modules: &Modules, database: &Database) -> Option<Finding> {
    match crate::modules::dormant_with_data(modules, database).await {
        Ok(dormant) if dormant.is_empty() => None,
        Ok(dormant) => {
            let (message, hint) = crate::modules::dormant_notice(modules, &dormant);
            let finding = if modules.selection_is_explicit() {
                Finding::ok(message)
            } else {
                Finding::warning(message)
            };
            Some(finding.with_hint(hint))
        }
        Err(error) => Some(Finding::warning(format!(
            "cannot check for data of modules that are turned off: {error}"
        ))),
    }
}

/// API công khai qua `https` mà không gửi HSTS (bước 40 P1): gợi ý, không lỗi. Không có URL công
/// khai `https` thì không có bước này.
fn probe_hsts(
    values: &[autoxide_kernel::config::SettingValue],
    http: &autoxide_kernel::config::HttpConfig,
) -> Option<Finding> {
    let public = values
        .iter()
        .find(|value| value.setting.name == "AUTOXIDE_HTTP_PUBLIC_URL")
        .map(|value| value.display_value())?;
    if !public.starts_with("https://") {
        return None;
    }
    Some(if !http.security_headers.enabled || http.security_headers.hsts_seconds == 0 {
        Finding::warning(
            "The API is public over HTTPS but sends no Strict-Transport-Security header: a \
             browser may still try plain HTTP first.",
        )
        .with_hint(
            "HSTS is on by default with an `https` public URL: remove AUTOXIDE_HTTP_HSTS_SECONDS=0 \
             (or set it, for example `31536000`, one year) and keep \
             AUTOXIDE_HTTP_SECURITY_HEADERS on, once every client reaches the API over HTTPS.",
        )
    } else {
        Finding::ok(format!(
            "Strict-Transport-Security is sent with max-age={}.",
            http.security_headers.hsts_seconds
        ))
    })
}

/// Đếm rate limit trong database mà dùng SQLite (bước 40 P6): vẫn đúng nhưng vô ích — SQLite chỉ
/// cho một tiến trình máy chủ — và thêm lệnh ghi lên kết nối ghi duy nhất. Gợi ý, không lỗi.
fn probe_rate_limit_store(config: &autoxide_kernel::config::Config) -> Option<Finding> {
    let shared = config.http.rate_limit_store == autoxide_kernel::config::RateLimitStore::Database;
    let sqlite = config.database.url.backend() == autoxide_kernel::config::DatabaseBackend::Sqlite;
    (shared && sqlite).then(|| {
        Finding::warning(
            "AUTOXIDE_HTTP_RATE_LIMIT_STORE=database with SQLite: SQLite runs one server process, \
             so counting in memory is just as exact and adds no writes.",
        )
        .with_hint("Use `memory` with SQLite; keep `database` for several servers on PostgreSQL.")
    })
}

fn probe_secret_key(modules: &Modules) -> Finding {
    match modules.api::<SecretKey>() {
        Some(key) if !key.is_configured() => Finding::warning(
            "AUTOXIDE_SECRET_KEY is not set: the root key is kept in the database, so a copy of \
             the database (a backup) holds the key too — it can open the secrets stored \
             encrypted (two-step sign-in, webhook signing secrets) and work visitors' IP blocks \
             back out of stored hashes.",
        )
        .with_hint("Set AUTOXIDE_SECRET_KEY (the same on every server) before going live."),
        _ => Finding::ok("The signing key is configured outside the database."),
    }
}

/// Báo cáo chữ: một dòng mỗi bước, gợi ý sửa thụt vào bên dưới.
fn render(lines: &[Line], healthy: bool) -> String {
    let mut out = String::new();
    for Line { name, status, message, hint } in lines {
        out.push_str(&format!("{status:<8} {name}: {message}\n"));
        if let Some(hint) = hint {
            out.push_str(&format!("         → {hint}\n"));
        }
    }
    let warnings = lines.iter().filter(|line| line.status == "warning").count();
    out.push_str(&match (healthy, warnings) {
        (true, 0) => "\nNo problems found.\n".to_owned(),
        (true, count) => {
            format!("\nNo failed checks; {count} warning(s) to look at before going live.\n")
        }
        (false, _) => {
            "\nFix the failed checks above, then run `autoxide doctor` again.\n".to_owned()
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use autoxide_kernel::config::ConfigSource;
    use autoxide_testkit::TestDatabase;
    use sea_orm::ConnectionTrait as _;

    use super::*;

    fn settings(database_url: &str, dir: &std::path::Path) -> Settings {
        let files = dir.join("files").display().to_string();
        let mail = dir.join("mail").display().to_string();
        let source = ConfigSource::from_pairs([
            ("DATABASE_URL", database_url),
            ("AUTOXIDE_FILES_DIR", files.as_str()),
            ("AUTOXIDE_MAIL_FILE_DIR", mail.as_str()),
        ]);
        crate::settings::read(&source).unwrap()
    }

    /// HSTS chỉ được nhắc khi API công khai qua `https` (bước 40 P1).
    #[test]
    fn nhac_hsts_khi_url_cong_khai_la_https() {
        let probe = |pairs: &[(&str, &str)]| {
            let source = ConfigSource::from_pairs(pairs.iter().copied());
            let settings = crate::settings::read(&source).unwrap();
            probe_hsts(&settings.values, &settings.config.http).map(|finding| finding.status)
        };
        assert_eq!(probe(&[]), None);
        assert_eq!(probe(&[("AUTOXIDE_HTTP_PUBLIC_URL", "http://localhost:8080")]), None);
        let https = ("AUTOXIDE_HTTP_PUBLIC_URL", "https://api.example.com");
        // Bước 71 G7: URL `https` tự bật HSTS; chỉ cảnh báo khi người vận hành tắt nó.
        assert_eq!(probe(&[https]), Some(Status::Ok));
        assert_eq!(probe(&[https, ("AUTOXIDE_HTTP_HSTS_SECONDS", "0")]), Some(Status::Warning));
        assert_eq!(
            probe(&[https, ("AUTOXIDE_HTTP_SECURITY_HEADERS", "false")]),
            Some(Status::Warning)
        );
        assert_eq!(probe(&[https, ("AUTOXIDE_HTTP_HSTS_SECONDS", "600")]), Some(Status::Ok));
    }

    /// `database` + SQLite: nhắc dùng `memory` (bước 40 P6).
    #[test]
    fn nhac_khi_dem_rate_limit_trong_sqlite() {
        let probe = |pairs: &[(&str, &str)]| {
            let source = ConfigSource::from_pairs(pairs.iter().copied());
            let settings = crate::settings::read(&source).unwrap();
            probe_rate_limit_store(&settings.config).map(|finding| finding.status)
        };
        let sqlite = ("DATABASE_URL", "sqlite://data/autoxide.sqlite");
        let shared = ("AUTOXIDE_HTTP_RATE_LIMIT_STORE", "database");
        assert_eq!(probe(&[sqlite]), None);
        assert_eq!(probe(&[sqlite, shared]), Some(Status::Warning));
        assert_eq!(probe(&[("DATABASE_URL", "postgres://u:p@localhost/db"), shared]), None);
    }

    fn statuses(lines: &[Line]) -> Vec<(&str, &str)> {
        lines.iter().map(|line| (line.name.as_str(), line.status)).collect()
    }

    /// Bản cài mới: báo migration chưa áp dụng (kèm lệnh), bỏ qua các bước sau. Áp dụng xong
    /// thì chạy bước kiểm của từng module; chỉ có cảnh báo thì vẫn đạt — trên cả hai database.
    #[tokio::test]
    async fn ban_cai_moi_roi_sau_khi_migrate() {
        TestDatabase::run(async |test| {
            let dir =
                std::env::temp_dir().join(format!("autoxide-doctor-{}", uuid::Uuid::now_v7()));

            let lines = diagnose(settings(test.url(), &dir)).await;
            assert_eq!(
                statuses(&lines),
                [
                    ("configuration", "ok"),
                    ("modules", "ok"),
                    ("database", "ok"),
                    ("migrations", "failed"),
                    ("secret_key", "skipped")
                ]
            );
            // Module đang chạy và đang tắt (bước 70 P1); test đọc cấu hình với mọi module.
            assert!(lines[1].message.contains("modules run: mail, ai,"), "{}", lines[1].message);
            assert!(lines[1].message.ends_with("No module is turned off."), "{}", lines[1].message);
            assert_eq!(lines[3].hint.as_deref(), Some("Run `autoxide migrate up`."));
            assert!(lines[3].message.contains(" more"), "{}", lines[3].message);

            let setup = settings(test.url(), &dir);
            autoxide_testkit::prepare_from_template(
                &setup.config.database,
                &setup.modules.migrator().unwrap(),
            )
            .await;
            let database = Database::connect(&setup.config.database).await.unwrap();
            setup.modules.migrator().unwrap().up(&database).await.unwrap();
            database.close().await.unwrap();

            let lines = diagnose(settings(test.url(), &dir)).await;
            let found = statuses(&lines);
            for expected in [
                ("migrations", "ok"),
                ("secret_key", "warning"),
                // Bản cài mới chưa có bản backup theo lịch nào (bước 39).
                ("kernel.backup_age", "warning"),
                ("auth.administrator", "warning"),
                // Chưa đặt khoá ký access token: máy chủ tự sinh khoá mỗi lần khởi động.
                ("auth.token_secret", "warning"),
                ("files.storage", "ok"),
                ("mail.smtp", "warning"),
                // Chưa nhập danh mục đơn vị hành chính (D-314).
                ("address.administrative_units", "warning"),
                // Gói thuế chưa nhập (job nhập trong vòng một giờ, D-341).
                ("tax.tax_rates", "warning"),
                ("ledger.chain", "ok"),
                ("ledger.balances", "ok"),
                // Chưa có website nào có hồ sơ Bộ Công Thương (D-357): cảnh báo, không hỏng.
                ("compliance.moit_readiness", "warning"),
                // Chưa nạp dữ liệu luật khuyến mại (bước 43): cảnh báo, không hỏng.
                ("promotions.promotion_limits", "warning"),
            ] {
                assert!(found.contains(&expected), "{expected:?} not in {found:?}");
            }
            assert!(lines.iter().all(|line| line.status != "failed"), "{lines:?}");
            let text = render(&lines, true);
            assert!(text.contains("→ Run `autoxide user create"), "{text}");
            assert!(text.contains("→ Set AUTOXIDE_AUTH_TOKEN_SECRET"), "{text}");
            assert!(
                text.ends_with("No failed checks; 9 warning(s) to look at before going live.\n"),
                "{text}"
            );

            // Database được một bản mới hơn migrate (bước 71 G3): báo `kernel.database_newer` và cách sửa riêng,
            // không phải "migration hỏng".
            let database = Database::connect(&setup.config.database).await.unwrap();
            let write = database.write();
            let sql = "INSERT INTO kernel_migrations (version, applied_at) \
                       VALUES ('m9999_from_a_newer_release', 0)";
            write
                .execute_raw(autoxide_kernel::database::statement(
                    write.get_database_backend(),
                    sql,
                    [],
                ))
                .await
                .unwrap();
            database.close().await.unwrap();
            let lines = diagnose(settings(test.url(), &dir)).await;
            let migrations = lines.iter().find(|line| line.name == "migrations").unwrap();
            assert_eq!(migrations.status, "failed");
            assert!(migrations.message.contains("m9999_from_a_newer_release"), "{migrations:?}");
            assert!(migrations.message.contains("newer Autoxide"), "{migrations:?}");
            let hint = migrations.hint.as_deref().unwrap_or_default();
            assert!(hint.contains("rollback --to"), "{hint}");
            assert!(newer_database_hint("bookmarks").contains("restore the backup"));
            let _ = std::fs::remove_dir_all(dir);
        })
        .await
        .unwrap();
    }

    #[test]
    fn json_cho_ai_agent() {
        let report = Report {
            healthy: false,
            checks: vec![
                line("migrations", Some(Finding::failed("1 pending").with_hint("Run it."))),
                line("secret_key", None),
            ],
        };
        assert_eq!(
            serde_json::to_value(&report).unwrap(),
            serde_json::json!({
                "healthy": false,
                "checks": [
                    {"name": "migrations", "status": "failed", "message": "1 pending", "hint": "Run it."},
                    {"name": "secret_key", "status": "skipped", "message": "Not run: an earlier check failed."}
                ]
            })
        );
    }
}
