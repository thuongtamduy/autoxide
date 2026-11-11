//! Binary `autoxide`: máy chủ và giao diện dòng lệnh (CLI).
//!
//! Đây là nơi DUY NHẤT ghép kernel với các module thành một chương trình. File
//! này chỉ phân tích dòng lệnh và chuyển cho từng lệnh trong `commands/`.

mod commands;
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
    /// Check the configuration and show every setting with its current value.
    ///
    /// Settings are read from environment variables and from a `.env` file in
    /// the current directory; environment variables win. Secrets are never
    /// printed. Exits with status 2 when the configuration is invalid.
    Config(commands::config::Arguments),

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
}

fn main() -> ExitCode {
    // `parse` tự xử lý `--help`, `--version` và tham số sai (in thông báo rồi thoát).
    let cli = Cli::parse();
    match cli.command {
        Command::Config(arguments) => commands::config::run(&arguments),
        Command::Migrate(arguments) => commands::migrate::run(&arguments),
        Command::Openapi => commands::openapi::run(),
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
