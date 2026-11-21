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
//!   (`i18n.rs`), tài liệu EN / VI (`docs.rs`), mục lục quyết định (`decisions.rs`), mảnh thay đổi
//!   (`changelog.rs`), lọc theo khách (`tenant_scope.rs`), tool MCP theo phạm vi
//!   (`mcp_principal.rs`; chỗ cũ chờ sửa ở `baseline.rs`) và độ dài
//!   file `.rs` (`lengths.rs`: quá 800 dòng thì cảnh báo, quá 1200 thì chặn trừ file miễn).
//!   `./gate.sh` gọi lệnh này; vi phạm thì gate đỏ.
//! - `changelog` — gộp các mảnh trong `changelog.d/` vào `## [Unreleased]` của `CHANGELOG.md` rồi
//!   xoá chúng; chạy lúc phát hành (xem `changelog.rs`, D-20261119-changelog-fragments).
//! - `decisions` — sinh lại mục lục `docs/decisions/INDEX.md` từ nhật ký quyết định (xem
//!   `decisions.rs`).
//! - `new-module <module> <entity>` — sinh module mới từ khuôn `modules/notes` (xem
//!   `scaffold.rs`).
//! - `test-all [--doc]` — mọi test của workspace, các file test chạy song song, trên database của
//!   `TEST_DATABASE_URL`; phần test của `./gate.sh` đầy đủ (xem `affected.rs`).
//! - `test-affected [--base <ref>]` — chạy test của crate bị sửa kể từ lần chạy xanh gần nhất
//!   (chưa có thì so với `main`) và crate phụ thuộc vào chúng, trên SQLite; phần test của
//!   `./gate.sh --quick` (xem `affected.rs`). `--base <ref> --keep-env`: trên database của
//!   `TEST_DATABASE_URL` thay vì SQLite; phần test của `./gate.sh --pr` (CI cho pull request).
//! - `test-filter --base <ref>` — in filterset của nextest chọn đúng các crate mà `test-affected
//!   --base <ref>` chạy; CI dùng khi chạy test từ archive (`./gate.sh --pr --only build`,
//!   D-20261120-ci-nextest-archive).
//! - Hai lệnh test trên nhận thêm `--shard <k>/<n>` ở cuối: chỉ chạy phần thứ k trong n phần của
//!   mọi test, để CI chạy n phần trên n máy cùng lúc (`./gate.sh --shard`, D-20261119-ci-faster).

mod affected;
mod api_diff;
mod baseline;
mod boundaries;
mod changelog;
mod decisions;
mod deploy_metrics;
mod docs;
mod i18n;
mod lengths;
mod mcp_principal;
mod metric_labels;
mod scaffold;
mod sdk;
mod semver;
mod source;
mod tenancy;
mod tenant_scope;
mod upgrade;
mod workspace;

use std::process::ExitCode;

const USAGE: &str = "usage: cargo xtask check
       cargo xtask decisions                     regenerate docs/decisions/INDEX.md
       cargo xtask changelog                     fold changelog.d/ fragments into CHANGELOG.md (at release)
       cargo xtask api-diff [--base <ref>]       default base: the last release tag before HEAD
       cargo xtask semver [--base <tag>] [--deny] [--markdown]
       cargo xtask sdk [--check | --extract-examples <dir>]
       cargo xtask upgrade-test [--from <tag>] [--image <image>] [--database sqlite|postgres|both]
       cargo xtask new-module <module> <entity>   e.g. new-module bookmarks bookmark
       cargo xtask test-affected [--base <ref>]   default base: the last green run, else origin/main
       cargo xtask test-affected --base <ref> --keep-env   on TEST_DATABASE_URL instead of SQLite
       cargo xtask test-all [--doc]               every test, on TEST_DATABASE_URL (default SQLite)
       cargo xtask test-filter --base <ref>       nextest filterset of the crates test-affected would run
       ... test-all | test-affected ... --shard <k>/<n>   only part k of n of the tests";

fn main() -> ExitCode {
    // `skip(1)`: phần tử đầu của args là tên chương trình.
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // `--shard k/n` ở cuối lệnh test: tách ra trước, phần còn lại so khớp như bình thường.
    let mut shard = affected::Shard::WHOLE;
    if args.len() >= 2 && args[args.len() - 2] == "--shard" {
        match affected::Shard::parse(&args[args.len() - 1]) {
            Some(parsed) => shard = parsed,
            None => {
                eprintln!("{USAGE}");
                return ExitCode::FAILURE;
            }
        }
        args.truncate(args.len() - 2);
        if !matches!(args.first().map(String::as_str), Some("test-all" | "test-affected")) {
            eprintln!("{USAGE}");
            return ExitCode::FAILURE;
        }
    }
    // `as_slice()` + pattern trên slice: so khớp theo số lượng và nội dung tham
    // số mà không phải tự kiểm độ dài.
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["check"] => check(),
        ["decisions"] => decisions(),
        ["changelog"] => changelog(),
        ["api-diff"] => api_diff(None),
        ["api-diff", "--base", base] => api_diff(Some(base)),
        ["semver", rest @ ..] => semver(rest),
        ["sdk"] => sdk(false),
        ["sdk", "--check"] => sdk(true),
        ["sdk", "--extract-examples", dir] => sdk_examples(dir),
        ["upgrade-test", rest @ ..] => upgrade_test(rest),
        ["new-module", module, entity] => new_module(module, entity),
        ["test-all"] => test_all(false, shard),
        ["test-all", "--doc"] => test_all(true, shard),
        ["test-affected"] => test_affected(None, false, shard),
        ["test-affected", "--base", base] => test_affected(Some(base), false, shard),
        ["test-affected", "--base", base, "--keep-env"] => test_affected(Some(base), true, shard),
        ["test-filter", "--base", base] => test_filter(base),
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
    let today = jiff::tz::Offset::UTC.to_datetime(jiff::Timestamp::now()).date();
    match api_diff::run(&root, base, today) {
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
        // Hai luật bảo mật có danh sách chỗ cũ chờ sửa (D-20261121-security-lints).
        let mut security = tenant_scope::check(&workspace)?;
        security.extend(mcp_principal::check(&workspace)?);
        let security = baseline::apply(&workspace.root, security)?;
        for warning in &security.warnings {
            eprintln!("{warning}");
        }
        violations.extend(security.violations);
        violations.extend(decisions::check(&workspace.root)?);
        violations.extend(changelog::check(&workspace.root)?);
        violations.extend(metric_labels::check(&workspace.root)?);
        violations.extend(deploy_metrics::check(&workspace.root)?);
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

fn changelog() -> ExitCode {
    let result = workspace::load()
        .map_err(|error| error.to_string())
        .and_then(|workspace| changelog::fold(&workspace.root));
    match result {
        Ok(count) => {
            println!(
                "changelog: folded {count} fragment(s) from {} into {}",
                changelog::DIR,
                changelog::CHANGELOG_FILE
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("xtask changelog: {error}");
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

fn test_all(doc: bool, shard: affected::Shard) -> ExitCode {
    match workspace::load().and_then(|workspace| affected::run_all(&workspace, doc, shard)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xtask test-all: {error}");
            ExitCode::FAILURE
        }
    }
}

fn test_affected(base: Option<&str>, keep_env: bool, shard: affected::Shard) -> ExitCode {
    let result =
        workspace::load().and_then(|workspace| affected::run(&workspace, base, keep_env, shard));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xtask test-affected: {error}");
            ExitCode::FAILURE
        }
    }
}

fn test_filter(base: &str) -> ExitCode {
    match workspace::load().and_then(|workspace| affected::filter(&workspace, base)) {
        Ok(filter) => {
            println!("{filter}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("xtask test-filter: {error}");
            ExitCode::FAILURE
        }
    }
}
