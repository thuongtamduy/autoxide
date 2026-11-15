//! Binary `autoxide`: máy chủ và giao diện dòng lệnh (CLI).
//!
//! Đây là nơi DUY NHẤT ghép kernel với các module thành một chương trình. File
//! này chỉ phân tích dòng lệnh và chuyển cho từng lệnh trong `commands/`.

mod commands;
#[cfg(test)]
mod e2e;
mod modules;
mod observability;
mod runtime;
mod settings;

use std::process::ExitCode;

use clap::{Parser, Subcommand};

/// Autoxide — a Rust backend core. Modules plug in on top.
// `version` lấy từ Cargo.toml lúc biên dịch; phần mô tả lấy từ dòng doc phía trên.
#[derive(Debug, Parser)]
#[command(name = "autoxide", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Check the AI settings against the provider (`autoxide ai check`).
    ///
    /// Sends one tiny request to the configured AI provider and embeddings service and says
    /// whether they work. Uses a few tokens.
    Ai(commands::ai::Arguments),

    /// Back up the data of every module and every stored file into a directory.
    ///
    /// Works the same for SQLite and PostgreSQL, local disk and S3: the backup holds a
    /// SQLite copy of the database (read in one consistent snapshot, so the server may
    /// keep running) and a copy of every stored file. Restore it with
    /// `autoxide restore`.
    Backup(commands::backup::BackupArguments),

    /// Check the configuration and show every setting with its current value.
    ///
    /// Settings are read from environment variables and from a `.env` file in
    /// the current directory; environment variables win. Secrets are never
    /// printed. Exits with status 2 when the configuration is invalid.
    Config(commands::config::Arguments),

    /// Copy the data of every module into another, empty database.
    ///
    /// Copies from this server's database (`DATABASE_URL`) to the target — SQLite to
    /// PostgreSQL or back. The target gets every migration first, must hold no data,
    /// and receives everything in one transaction. Stop the server first so that no
    /// change is missed.
    Copy(commands::copy::Arguments),

    /// Check this installation step by step and say how to fix what is wrong.
    ///
    /// Checks the configuration, the database (reading and writing), the migrations, the
    /// signing key, then what each module declares: file storage, the SMTP server, whether
    /// an administrator exists… Changes nothing (a test file written to the storage is
    /// deleted again). Exits with status 1 when a check failed, 2 when the configuration is
    /// invalid; warnings do not fail.
    Doctor(commands::doctor::Arguments),

    /// Inspect and retry background jobs that failed for good (the dead-letter queue).
    Jobs(commands::jobs::Arguments),

    /// Show, apply or revert the database migrations of the modules.
    ///
    /// Migrations are never applied automatically: run `autoxide migrate up` once
    /// per deployment, before starting the server.
    Migrate(commands::migrate::Arguments),

    /// Print the OpenAPI document of the whole API as JSON.
    ///
    /// The same document is served at `/api/v1/openapi.json`. Connects to the
    /// database (modules build their services) but does not need migrations.
    Openapi,

    /// Print the contract of every MCP tool (name, permissions, input schema, deprecation) as JSON.
    ///
    /// Kept in `docs/mcp-tools.json`; `cargo xtask api-diff` refuses breaking changes to it.
    /// Connects to the database (modules build their services) but does not need migrations.
    McpManifest,

    /// Restore a backup made by `autoxide backup` into a new, empty database.
    ///
    /// Restores into the database of `DATABASE_URL` (which gets every migration first)
    /// and the file storage configured now — they may differ from the backed-up server.
    Restore(commands::backup::RestoreArguments),

    /// Run the HTTP server until it receives Ctrl-C or SIGTERM.
    ///
    /// Refuses to start while a migration has not been applied: run
    /// `autoxide migrate up` first. On shutdown, it first keeps handling requests for
    /// `AUTOXIDE_HTTP_DRAIN_SECONDS` while `/health/ready` answers 503, then stops accepting
    /// connections and gives requests in progress up to `AUTOXIDE_HTTP_SHUTDOWN_SECONDS` to
    /// finish.
    Serve,

    /// List or run the maintenance tasks that modules declare.
    ///
    /// `autoxide task` lists every task with its arguments; `autoxide task <module>.<task>
    /// [name=value …]` runs one, e.g. `autoxide task media.refresh_variants dry_run=true`.
    /// Runs with the rights of whoever operates the server. Exits with status 64 when the task
    /// or an argument is wrong, 1 when the task fails. Run `autoxide migrate up` first.
    Task(commands::task::Arguments),

    /// Encrypt stored secrets again with the current key (after rotating AUTOXIDE_SECRET_KEY).
    ///
    /// `autoxide secrets reseal` runs the `reseal_secrets` task of every module that keeps
    /// encrypted secrets. Run `autoxide migrate up` first.
    Secrets(commands::secrets::Arguments),

    /// Manage the tenants (customers) of the installation: create, list, show, suspend, resume.
    ///
    /// Runs with the rights of whoever operates the server. Creating tenants needs
    /// AUTOXIDE_TENANCY_MODE=multi.
    Tenant(commands::tenant::Arguments),

    /// Manage user accounts, e.g. create the first administrator.
    ///
    /// Runs with the rights of whoever operates the server: no sign-in needed.
    User(commands::user::Arguments),
}

fn main() -> ExitCode {
    // `parse` tự xử lý `--help`, `--version` và tham số sai (in thông báo rồi thoát).
    let cli = Cli::parse();
    match cli.command {
        Command::Ai(arguments) => commands::ai::run(&arguments),
        Command::Backup(arguments) => commands::backup::backup(&arguments),
        Command::Config(arguments) => commands::config::run(&arguments),
        Command::Copy(arguments) => commands::copy::run(&arguments),
        Command::Doctor(arguments) => commands::doctor::run(&arguments),
        Command::Jobs(arguments) => commands::jobs::run(&arguments),
        Command::Migrate(arguments) => commands::migrate::run(&arguments),
        Command::Openapi => commands::openapi::run(),
        Command::McpManifest => commands::mcp_manifest::run(),
        Command::Restore(arguments) => commands::backup::restore(&arguments),
        Command::Secrets(arguments) => commands::secrets::run(&arguments),
        Command::Serve => commands::serve::run(),
        Command::Task(arguments) => commands::task::run(&arguments),
        Command::Tenant(arguments) => commands::tenant::run(&arguments),
        Command::User(arguments) => commands::user::run(&arguments),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use clap::error::ErrorKind;

    #[test]
    fn khai_bao_cli_hop_le() {
        // clap tự kiểm các khai báo mâu thuẫn (trùng tên tham số, thiếu giá trị mặc định…).
        Cli::command().debug_assert();
    }

    #[test]
    fn version_lay_tu_cargo_toml() {
        let error = Cli::try_parse_from(["autoxide", "--version"]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::DisplayVersion);
        assert_eq!(error.to_string().trim(), concat!("autoxide ", env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn tham_so_la_bi_tu_choi() {
        let error = Cli::try_parse_from(["autoxide", "--no-such-flag"]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnknownArgument);
    }

    #[test]
    fn khong_co_lenh_thi_in_huong_dan() {
        let error = Cli::try_parse_from(["autoxide"]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand);
    }
}
