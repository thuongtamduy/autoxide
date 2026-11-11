//! Công cụ cho người phát triển Autoxide. Chạy bằng `cargo xtask <lệnh>`.
//!
//! Hiện có một lệnh: `check` — kiểm luật ranh giới của workspace (xem
//! `boundaries.rs`). `./gate.sh` gọi lệnh này; vi phạm thì gate đỏ.

mod boundaries;
mod workspace;

use std::process::ExitCode;

const USAGE: &str = "usage: cargo xtask check";

fn main() -> ExitCode {
    // `skip(1)`: phần tử đầu của args là tên chương trình.
    let args: Vec<String> = std::env::args().skip(1).collect();
    // `as_slice()` + pattern trên slice: so khớp theo số lượng và nội dung tham
    // số mà không phải tự kiểm độ dài.
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["check"] => check(),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::FAILURE
        }
    }
}

fn check() -> ExitCode {
    let workspace = match workspace::load() {
        Ok(workspace) => workspace,
        Err(error) => {
            eprintln!("xtask check: {error}");
            return ExitCode::FAILURE;
        }
    };
    let violations = match boundaries::check_workspace(&workspace) {
        Ok(violations) => violations,
        Err(error) => {
            eprintln!("xtask check: {error}");
            return ExitCode::FAILURE;
        }
    };
    if violations.is_empty() {
        println!("xtask check: {} crates, no boundary violations", workspace.packages.len());
        return ExitCode::SUCCESS;
    }
    for violation in &violations {
        eprintln!("{violation}");
    }
    eprintln!("xtask check: {} violation(s)", violations.len());
    ExitCode::FAILURE
}
