//! Công cụ cho người phát triển Autoxide. Chạy bằng `cargo xtask <lệnh>`.
//!
//! - `api-diff [--base <ref>]` — chặn thay đổi phá vỡ của `/api/v1` so với bản phát hành gần
//!   nhất (xem `api_diff.rs`). `./gate.sh` gọi lệnh này ở cả hai mức.
//! - `semver [--base <tag>] [--deny] [--markdown]` — báo thay đổi phá vỡ của hợp đồng Rust
//!   (kernel, testkit) bằng `cargo semver-checks`; chạy lúc phát hành (xem `semver.rs`).
//! - `sdk [--check | --extract-examples <dir>]` — sinh kiểu TypeScript của SDK từ
//!   `docs/openapi.json`; `--check` (trong gate) báo khi file đã commit lệch (xem `sdk.rs`).
//! - `upgrade-test [--from <tag>] [--image <image>] [--database …]` — test nâng cấp từ bản phát
//!   hành trước bằng image Docker của nó; chạy trong CI và lúc phát hành (xem `upgrade.rs`).
//! - `check` — kiểm luật ranh giới của workspace (xem `boundaries.rs`), bản dịch i18n
//!   (`i18n.rs`), tài liệu EN / VI (`docs.rs`), mục lục quyết định (`decisions.rs`) và độ dài
//!   file `.rs` (`lengths.rs`: quá 800 dòng thì cảnh báo, quá 1200 thì chặn trừ file miễn).
//!   `./gate.sh` gọi lệnh này; vi phạm thì gate đỏ.
//! - `decisions` — sinh lại mục lục `docs/decisions/INDEX.md` từ nhật ký quyết định (xem
//!   `decisions.rs`).
//! - `new-module <module> <entity>` — sinh module mới từ khuôn `modules/notes` (xem
//!   `scaffold.rs`).
//! - `test-all [--doc]` — mọi test của workspace, các file test chạy song song, trên database của
//!   `TEST_DATABASE_URL`; phần test của `./gate.sh` đầy đủ (xem `affected.rs`).
//! - `test-affected [--base <ref>]` — chạy test của crate bị sửa kể từ lần chạy xanh gần nhất
//!   (chưa có thì so với `main`) và crate phụ thuộc vào chúng, trên SQLite; phần test của
//!   `./gate.sh --quick` (xem `affected.rs`).

mod affected;
mod api_diff;
mod boundaries;
mod decisions;
mod docs;
mod i18n;
mod lengths;
mod scaffold;
mod sdk;
mod semver;
mod tenancy;
mod upgrade;
mod workspace;

use std::process::ExitCode;

const USAGE: &str = "usage: cargo xtask check
       cargo xtask decisions                     regenerate docs/decisions/INDEX.md
       cargo xtask api-diff [--base <ref>]       default base: the last release tag before HEAD
       cargo xtask semver [--base <tag>] [--deny] [--markdown]
       cargo xtask sdk [--check | --extract-examples <dir>]
       cargo xtask upgrade-test [--from <tag>] [--image <image>] [--database sqlite|postgres|both]
       cargo xtask new-module <module> <entity>   e.g. new-module bookmarks bookmark
       cargo xtask test-affected [--base <ref>]   default base: the last green run, else origin/main
       cargo xtask test-all [--doc]               every test, on TEST_DATABASE_URL (default SQLite)";

fn main() -> ExitCode {
    // `skip(1)`: phần tử đầu của args là tên chương trình.
    let args: Vec<String> = std::env::args().skip(1).collect();
    // `as_slice()` + pattern trên slice: so khớp theo số lượng và nội dung tham
    // số mà không phải tự kiểm độ dài.
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["check"] => check(),
        ["decisions"] => decisions(),
        ["api-diff"] => api_diff(None),
        ["api-diff", "--base", base] => api_diff(Some(base)),
        ["semver", rest @ ..] => semver(rest),
        ["sdk"] => sdk(false),
        ["sdk", "--check"] => sdk(true),
        ["sdk", "--extract-examples", dir] => sdk_examples(dir),
        ["upgrade-test", rest @ ..] => upgrade_test(rest),
        ["new-module", module, entity] => new_module(module, entity),
        ["test-all"] => test_all(false),
        ["test-all", "--doc"] => test_all(true),
        ["test-affected"] => test_affected(None),
        ["test-affected", "--base", base] => test_affected(Some(base)),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::FAILURE
        }
    }
}

fn api_diff(base: Option<&str>) -> ExitCode {
    let root = match workspace::load() {
        Ok(workspace) => workspace.root,
        Err(error) => {
            eprintln!("xtask api-diff: {error}");
            return ExitCode::FAILURE;
        }
    };
    match api_diff::run(&root, base, chrono::Utc::now().date_naive()) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("xtask api-diff: {error}");
            ExitCode::FAILURE
        }
    }
}

fn semver(args: &[&str]) -> ExitCode {
    let mut base = None;
    let (mut deny, mut markdown) = (false, false);
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match *arg {
            "--deny" => deny = true,
            "--markdown" => markdown = true,
            "--base" => base = args.next().copied(),
            _ => {
                eprintln!("{USAGE}");
                return ExitCode::FAILURE;
            }
        }
    }
    let root = match workspace::load() {
        Ok(workspace) => workspace.root,
        Err(error) => {
            eprintln!("xtask semver: {error}");
            return ExitCode::FAILURE;
        }
    };
    match semver::run(&root, base, deny, markdown) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("xtask semver: {error}");
            ExitCode::FAILURE
        }
    }
}

fn sdk(check: bool) -> ExitCode {
    let result = workspace::load().map_err(|error| error.to_string()).and_then(|workspace| {
        let version = semver::workspace_version(&workspace.root)?;
        sdk::run(&workspace.root, check, &version)
    });
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("xtask sdk: {error}");
            ExitCode::FAILURE
        }
    }
}

fn upgrade_test(args: &[&str]) -> ExitCode {
    let result = upgrade::parse(args).and_then(|options| {
        let workspace = workspace::load().map_err(|error| error.to_string())?;
        upgrade::run(&workspace.root, &options)
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xtask upgrade-test: {error}");
            ExitCode::FAILURE
        }
    }
}

fn sdk_examples(dir: &str) -> ExitCode {
    let result = workspace::load()
        .map_err(|error| error.to_string())
        .and_then(|workspace| sdk::extract_examples(&workspace.root, std::path::Path::new(dir)));
    match result {
        Ok(count) => {
            println!("sdk: wrote {count} TypeScript example(s) to {dir}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("xtask sdk: {error}");
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
        violations.extend(tenancy::check_entities(&workspace)?);
        violations.extend(decisions::check(&workspace.root)?);
        let lengths = lengths::check(&workspace.root)?;
        if let Some(warning) = lengths.warning {
            eprintln!("{warning}");
        }
        violations.extend(lengths.violations);
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

fn decisions() -> ExitCode {
    let result = workspace::load()
        .map_err(|error| error.to_string())
        .and_then(|workspace| decisions::write_index(&workspace.root));
    match result {
        Ok(count) => {
            println!("decisions: wrote {} ({count} decisions)", decisions::INDEX_FILE);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("xtask decisions: {error}");
            ExitCode::FAILURE
        }
    }
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

fn test_all(doc: bool) -> ExitCode {
    match workspace::load().and_then(|workspace| affected::run_all(&workspace, doc)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xtask test-all: {error}");
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
