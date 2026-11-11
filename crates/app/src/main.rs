//! Binary `autoxide`: máy chủ và giao diện dòng lệnh (CLI).
//!
//! Đây là nơi DUY NHẤT ghép kernel với các module thành một chương trình. Ở bước 0
//! mới có khung CLI (`--help`, `--version`); các lệnh `serve` và `migrate` được
//! thêm ở bước 1 của `docs/ROADMAP.md`.

use std::process::ExitCode;

use clap::{CommandFactory, Parser};

/// Autoxide — a Rust backend core. Modules plug in on top.
// `version` lấy từ Cargo.toml lúc biên dịch; `about` lấy từ dòng doc phía trên.
#[derive(Debug, Parser)]
#[command(name = "autoxide", version, about)]
struct Cli {}

fn main() -> ExitCode {
    // `parse` tự xử lý `--help`, `--version` và tham số sai (in thông báo rồi thoát).
    let Cli {} = Cli::parse();

    // Chưa có lệnh con nào: in hướng dẫn thay vì im lặng thoát.
    match Cli::command().print_help() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("autoxide: cannot write to the terminal: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
