//! Công cụ cho người phát triển Autoxide. Chạy bằng `cargo xtask <lệnh>`.
//!
//! - `check` — kiểm luật ranh giới của workspace (xem `boundaries.rs`), bản dịch i18n
//!   (`i18n.rs`) và tài liệu EN / VI (`docs.rs`). `./gate.sh` gọi lệnh này; vi phạm thì gate đỏ.
//! - `new-module <module> <entity>` — sinh module mới từ khuôn `modules/notes` (xem
//!   `scaffold.rs`).
//! - `test-affected [--base <ref>]` — chạy test của crate bị sửa so với `main` và crate phụ
//!   thuộc vào chúng, trên SQLite; phần test của `./gate.sh --quick` (xem `affected.rs`).

mod affected;
mod boundaries;
mod docs;
mod i18n;
mod scaffold;
mod workspace;

use std::process::ExitCode;

const USAGE: &str = "usage: cargo xtask check
       cargo xtask new-module <module> <entity>   e.g. new-module bookmarks bookmark
       cargo xtask test-affected [--base <ref>]   default base: origin/main";

fn main() -> ExitCode {
    // `skip(1)`: phần tử đầu của args là tên chương trình.
    let args: Vec<String> = std::env::args().skip(1).collect();
    // `as_slice()` + pattern trên slice: so khớp theo số lượng và nội dung tham
    // số mà không phải tự kiểm độ dài.
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["check"] => check(),
        ["new-module", module, entity] => new_module(module, entity),
        ["test-affected"] => test_affected(None),
        ["test-affected", "--base", base] => test_affected(Some(base)),
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
    let violations = match boundaries::check_workspace(&workspace).and_then(|mut violations| {
        violations.extend(i18n::check_translations(&workspace)?);
        violations.extend(docs::check_docs(&workspace.root)?);
        Ok(violations)
    }) {
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

fn new_module(module: &str, entity: &str) -> ExitCode {
    let workspace = match workspace::load() {
        Ok(workspace) => workspace,
        Err(error) => {
            eprintln!("xtask new-module: {error}");
            return ExitCode::FAILURE;
        }
    };
    match scaffold::new_module(&workspace.root, module, entity) {
        Ok(written) => {
            println!(
                "Created modules/{module} ({} files) from the `notes` template.",
                written.len()
            );
            println!();
            println!("Next:");
            println!(
                "  1. cargo test -p autoxide-{module}       (runs the copied tests on SQLite)"
            );
            println!(
                "  2. Rewrite the tables, service, descriptions and locales/vi.json for your feature."
            );
            println!("  3. Register it (docs/MODULE_GUIDE.md, step 1):");
            println!(
                "       Cargo.toml:            autoxide-{module} = {{ path = \"modules/{module}\" }}"
            );
            println!("       crates/app/Cargo.toml: autoxide-{module}.workspace = true");
            println!("       crates/app/src/modules.rs: Box::new(autoxide_{module}::module()),");
            println!("  4. ./gate.sh");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("xtask new-module: {error}");
            ExitCode::FAILURE
        }
    }
}

fn test_affected(base: Option<&str>) -> ExitCode {
    let result = workspace::load().and_then(|workspace| affected::run(&workspace, base));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xtask test-affected: {error}");
            ExitCode::FAILURE
        }
    }
}
