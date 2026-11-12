//! Binary `autoxide`: máy chủ và giao diện dòng lệnh (CLI).
//!
//! Đây là nơi DUY NHẤT ghép kernel với các module thành một chương trình. File
//! này chỉ phân tích dòng lệnh và chuyển cho từng lệnh trong `commands/`.

mod commands;
#[cfg(test)]
mod media_e2e;
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

    /// Restore a backup made by `autoxide backup` into a new, empty database.
    ///
    /// Restores into the database of `DATABASE_URL` (which gets every migration first)
    /// and the file storage configured now — they may differ from the backed-up server.
    Restore(commands::backup::RestoreArguments),

    /// Run the HTTP server until it receives Ctrl-C or SIGTERM.
    ///
    /// Refuses to start while a migration has not been applied: run
    /// `autoxide migrate up` first. On shutdown, requests in progress get up to
    /// `AUTOXIDE_HTTP_SHUTDOWN_SECONDS` to finish.
    Serve,

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
        Command::Jobs(arguments) => commands::jobs::run(&arguments),
        Command::Migrate(arguments) => commands::migrate::run(&arguments),
        Command::Openapi => commands::openapi::run(),
        Command::Restore(arguments) => commands::backup::restore(&arguments),
        Command::Serve => commands::serve::run(),
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
