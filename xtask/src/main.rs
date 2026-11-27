//! Công cụ cho người phát triển Autoxide. Chạy bằng `cargo xtask <lệnh>`.
//!
//! - `api-diff [--base <ref>]` — chặn thay đổi phá vỡ của `/api/v1` so với bản phát hành gần
//!   nhất (xem `api_diff.rs`). `./gate.sh` gọi lệnh này ở cả hai mức.
//! - `semver [--base <rev>] [--changed <rev>] [--deny] [--markdown] [--shard k/n]` — báo thay đổi phá vỡ của hợp
//!   đồng Rust (kernel, testkit, `autoxide`, module chính thức) bằng `cargo semver-checks`; chạy ở job
//!   CI `semver` và lúc phát hành (xem `semver.rs`).
//! - `sdk [--check | --extract-examples <dir>]` — sinh kiểu TypeScript của SDK từ
//!   `docs/openapi.json`; `--check` (trong gate) báo khi file đã commit lệch (xem `sdk.rs`).
//! - `upgrade-test [--from <tag>] [--image <image>] [--database …]` — test nâng cấp từ bản phát
//!   hành trước bằng image Docker của nó; chạy trong CI và lúc phát hành (xem `upgrade.rs`).
//! - `check` — kiểm luật ranh giới của workspace (xem `boundaries.rs`), bản dịch i18n
//!   (`i18n.rs`), tài liệu EN / VI (`docs.rs`), doc comment tiếng Anh của phần công khai
//!   (`doc_language.rs`), mục lục quyết định (`decisions.rs`), mảnh thay đổi
//!   (`changelog.rs`), lọc theo khách (`tenant_scope.rs`), tool MCP theo phạm vi
//!   (`mcp_principal.rs`; chỗ cũ chờ sửa ở `baseline.rs`), handler kiểm quyền trước khi đọc tham số trang
//!   (`permission_first.rs`) và độ dài
//!   file `.rs` (`lengths.rs`: quá 800 dòng thì cảnh báo, quá 1200 thì chặn trừ file miễn), hình
//!   dạng thống nhất của API trên `docs/openapi.json` (`api_shape.rs`: thân request đóng, lỗi
//!   `Problem`, danh sách `Page<T>`).
//!   `./gate.sh` gọi lệnh này; vi phạm thì gate đỏ.
//! - `doctor` — máy của người phát triển đã đủ công cụ chưa, kèm lệnh cài từng thứ thiếu (xem `doctor.rs`).
//! - `check-module <thư mục>` — các luật của `check` áp cho một module, chạy trên crate nằm ngoài
//!   repo này (module ở repo riêng; xem `external.rs`).
//! - `publish-check [--no-verify]` — đóng gói mọi crate như `cargo publish` rồi build thử
//!   từng gói; chạy lúc phát hành, không đăng gì (xem `publish.rs`). Luật `publish-ready` của `check`
//!   giữ metadata và phiên bản phụ thuộc nội bộ.
//! - `migration-releases record <X.Y.Z> [--from <file>]` — ghi các migration lần đầu có ở bản X.Y.Z vào
//!   `crates/app/migration-releases.json` cho `autoxide rollback`; chạy lúc phát hành (xem `migration_releases.rs`).
//! - `changelog` — gộp các mảnh trong `changelog.d/` vào `## [Unreleased]` của `CHANGELOG.md` rồi
//!   xoá chúng; chạy lúc phát hành (xem `changelog.rs`, D-20261119-changelog-fragments).
//! - `coverage-api [--run]` — endpoint trong `docs/openapi.json` và tool trong `docs/mcp-tools.json` chưa
//!   được test nào gọi thành công; `--run` chạy `test-all` để đo trước (xem `coverage.rs`).
//! - `decisions` — sinh lại mục lục `docs/decisions/INDEX.md` từ nhật ký quyết định (xem
//!   `decisions.rs`).
//! - `new-module <module> <entity>` — sinh module mới từ khuôn `modules/notes` (xem
//!   `scaffold.rs`). `new-module --external <dir> <module> <entity>
//!   [--core-path <dir>]` — sinh một workspace Cargo riêng ngoài repo (module, binary, README, AGENTS.md,
//!   CI mẫu) từ `examples/external-module`; `--core-path` trỏ core vào một bản checkout thay git tag.
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

mod address_keys;
mod affected;
mod api_diff;
mod api_shape;
mod baseline;
mod boundaries;
mod changelog;
mod coverage;
mod decisions;
mod deploy_metrics;
mod doc_language;
mod docs;
mod doctor;
mod external;
mod hints;
mod i18n;
mod lengths;
mod library_layer;
mod mcp_principal;
mod metric_labels;
mod migration_releases;
mod permission_first;
mod publish;
mod regen;
mod rust_free;
mod scaffold;
mod sdk;
mod semver;
mod site_links;
mod source;
mod tenancy;
mod tenant_scope;
mod toolchain;
mod upgrade;
mod workspace;

use std::process::ExitCode;

const USAGE: &str = "usage: cargo xtask check
       cargo xtask doctor                        is this machine ready to work on Autoxide? (read-only)
       cargo xtask check-module <dir>            the module rules of `check`, on a crate outside this repository
       cargo xtask coverage-api [--run]          endpoints and MCP tools no test calls successfully
       cargo xtask decisions                     regenerate docs/decisions/INDEX.md
       cargo xtask regen [--check]               regenerate every generated file (openapi, mcp-tools, .env.example, SDK, decisions index)
       cargo xtask changelog                     fold changelog.d/ fragments into CHANGELOG.md (at release)
       cargo xtask migration-releases record <X.Y.Z> [--from <file>]   record the migrations of a release (at release)
       cargo xtask api-diff [--base <ref>]       default base: the last release tag before HEAD
       cargo xtask semver [--base <rev>] [--changed <rev>] [--deny] [--markdown] [--shard <k>/<n>]
       cargo xtask publish-check [--no-verify]   package every crate as for crates.io (publishes nothing)
       cargo xtask sdk [--check | --extract-examples <dir>]
       cargo xtask upgrade-test [--from <tag>] [--image <image>] [--database sqlite|postgres|both]
       cargo xtask new-module <module> <entity>   e.g. new-module bookmarks bookmark
       cargo xtask new-module --external <dir> <module> <entity> [--core-path <autoxide checkout>]
                                                   a workspace of your own, outside this repository
       cargo xtask test-affected [--base <ref>]   default base: the last green run, else origin/main
       cargo xtask test-affected --base <ref> --keep-env   on TEST_DATABASE_URL instead of SQLite
       cargo xtask test-all [--doc]               every test, on TEST_DATABASE_URL (default SQLite)
       cargo xtask test-filter --base <ref>       nextest filterset of the crates test-affected would run
       ... test-all | test-affected ... --shard <k>/<n>   only part k of n of the tests
       ... semver ... --shard <k>/<n>             only part k of n of the contract crates";

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
        if !matches!(
            args.first().map(String::as_str),
            Some("test-all" | "test-affected" | "semver")
        ) {
            eprintln!("{USAGE}");
            return ExitCode::FAILURE;
        }
    }
    // `as_slice()` + pattern trên slice: so khớp theo số lượng và nội dung tham
    // số mà không phải tự kiểm độ dài.
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["check"] => check(),
        ["doctor"] => doctor::run(),
        ["check-module", dir] => check_module(dir),
        ["coverage-api"] => coverage_api(false),
        ["coverage-api", "--run"] => coverage_api(true),
        ["decisions"] => decisions(),
        ["regen"] => regen(false),
        ["regen", "--check"] => regen(true),
        ["changelog"] => changelog(),
        ["migration-releases", "record", version] => migration_releases(version, None),
        ["migration-releases", "record", version, "--from", file] => {
            migration_releases(version, Some(file))
        }
        ["api-diff"] => api_diff(None),
        ["api-diff", "--base", base] => api_diff(Some(base)),
        ["semver", rest @ ..] => semver(rest, shard),
        ["publish-check", rest @ ..] => publish_check(rest),
        ["sdk"] => sdk(false),
        ["sdk", "--check"] => sdk(true),
        ["sdk", "--extract-examples", dir] => sdk_examples(dir),
        ["upgrade-test", rest @ ..] => upgrade_test(rest),
        ["new-module", module, entity] => new_module(module, entity),
        ["new-module", "--external", dir, module, entity] => {
            new_external(dir, module, entity, None)
        }
        ["new-module", "--external", dir, module, entity, "--core-path", core] => {
            new_external(dir, module, entity, Some(core))
        }
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

fn semver(args: &[&str], shard: affected::Shard) -> ExitCode {
    let (mut base, mut changed) = (None, None);
    let (mut deny, mut markdown) = (false, false);
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match *arg {
            "--deny" => deny = true,
            "--markdown" => markdown = true,
            "--base" => base = args.next().copied(),
            "--changed" => changed = args.next().copied(),
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
    match semver::run(&root, base, changed, deny, markdown, shard) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("xtask semver: {error}");
            ExitCode::FAILURE
        }
    }
}

fn publish_check(args: &[&str]) -> ExitCode {
    let verify = match args {
        [] => true,
        ["--no-verify"] => false,
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::FAILURE;
        }
    };
    match workspace::load().and_then(|workspace| publish::run(&workspace.root, verify)) {
        Ok(()) => {
            println!(
                "xtask publish-check: every crate packages for crates.io (nothing was published)"
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("xtask publish-check: {error}");
            ExitCode::FAILURE
        }
    }
}

/// `cargo xtask regen [--check]` (xem `regen.rs`).
fn regen(check: bool) -> ExitCode {
    let result = workspace::load().map_err(|error| error.to_string()).and_then(|workspace| {
        let stale = regen::outputs(&workspace.root, check)?;
        if check {
            for file in &stale {
                eprintln!("{}", regen::stale_message(file));
            }
            return Ok(stale.is_empty());
        }
        for file in &stale {
            println!("regen: rewrote {file}");
        }
        // SDK đọc `docs/openapi.json` vừa ghi; mục lục quyết định không nằm trong git.
        let version = semver::workspace_version(&workspace.root)?;
        let sdk_ok = sdk::run(&workspace.root, false, &version)?;
        decisions::write_index(&workspace.root)?;
        Ok(sdk_ok)
    });
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("xtask regen: {error}");
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

fn check_module(dir: &str) -> ExitCode {
    match external::check(std::path::Path::new(dir)) {
        Ok((module, violations)) if violations.is_empty() => {
            println!("xtask check-module: module `{module}`, no violations");
            ExitCode::SUCCESS
        }
        Ok((_, violations)) => {
            for violation in &violations {
                eprintln!("{violation}");
            }
            eprintln!("xtask check-module: {} violation(s)", violations.len());
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("xtask check-module: {error}");
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
        // Doc comment của phần công khai viết tiếng Anh (D-20261123-english-rustdoc).
        violations.extend(doc_language::check(&workspace.root)?);
        violations.extend(publish::check(&workspace)?);
        violations.extend(tenancy::check_entities(&workspace)?);
        // Hai luật bảo mật có danh sách chỗ cũ chờ sửa (D-20261121-security-lints).
        let mut security = tenant_scope::check(&workspace)?;
        security.extend(mcp_principal::check(&workspace)?);
        let security = baseline::apply(&workspace.root, security)?;
        for warning in &security.warnings {
            eprintln!("{warning}");
        }
        violations.extend(security.violations);
        // Lớp kernel thay thư viện (D-20261126-library-isolation): chỗ cũ chờ chuyển ở `library-baseline.txt`.
        let layer = library_layer::check(&workspace)?;
        let layer = baseline::apply_file(&workspace.root, library_layer::BASELINE, layer)?;
        for warning in &layer.warnings {
            eprintln!("{warning}");
        }
        violations.extend(layer.violations);
        violations.extend(library_layer::check_private_paths(&workspace)?);
        violations.extend(library_layer::check_single_copies(&workspace.root)?);
        violations.extend(library_layer::check_strict(&workspace)?);
        violations.extend(decisions::check(&workspace.root)?);
        violations.extend(changelog::check(&workspace.root)?);
        violations.extend(metric_labels::check(&workspace.root)?);
        // Khoá giới hạn theo địa chỉ client gom theo khối (D-20261125-address-keys).
        violations.extend(address_keys::check(&workspace.root)?);
        // Link giao diện trong thư theo khách (D-20261125-tenant-link-templates).
        violations.extend(site_links::check(&workspace.root)?);
        violations.extend(deploy_metrics::check(&workspace.root)?);
        // Handler kiểm quyền trước khi đọc tham số trang (D-20261126-permission-before-page).
        violations.extend(permission_first::check(&workspace.root)?);
        violations.extend(toolchain::check(&workspace.root)?);
        // Danh sách file mà PR chỉ đổi chúng thì CI bỏ phần Rust (D-20261125-ci-skip-rust).
        violations.extend(rust_free::check(&workspace)?);
        // Hình dạng thống nhất của API trên docs/openapi.json (D-20261121-api-shape).
        let shape = api_shape::check(&workspace.root)?;
        for warning in &shape.warnings {
            eprintln!("{warning}");
        }
        violations.extend(shape.violations);
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

fn migration_releases(version: &str, from: Option<&str>) -> ExitCode {
    let result = workspace::load()
        .map_err(|error| error.to_string())
        .and_then(|workspace| migration_releases::record(&workspace.root, version, from));
    match result {
        Ok(added) => {
            println!(
                "migration-releases: {version}: {added} new migration(s) in {}",
                migration_releases::FILE
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("xtask migration-releases: {error}");
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

fn new_external(dir: &str, module: &str, entity: &str, core_path: Option<&str>) -> ExitCode {
    let result = workspace::load().and_then(|workspace| {
        let version = semver::workspace_version(&workspace.root)?;
        let core_dir = match core_path {
            Some(path) => Some(
                std::fs::canonicalize(path)
                    .map_err(|error| format!("--core-path {path}: {error}"))?,
            ),
            None => None,
        };
        let core = match &core_dir {
            Some(path) => scaffold::CoreSource::Path(path),
            None => scaffold::CoreSource::Tag(&version),
        };
        let written = scaffold::new_external(
            &workspace.root,
            std::path::Path::new(dir),
            module,
            entity,
            &core,
        )?;
        // Tên mới dài / ngắn hơn tên khuôn làm lệch định dạng (thứ tự import, độ dài dòng): định dạng
        // lại để khung sinh ra qua `cargo fmt --check` của `check.sh` ngay. Không có rustfmt thì chỉ báo.
        let formatted = std::process::Command::new("cargo")
            .args(["fmt", "--all"])
            .current_dir(dir)
            .status()
            .is_ok_and(|status| status.success());
        if !formatted {
            eprintln!("xtask new-module: `cargo fmt --all` failed in {dir}; run it there yourself");
        }
        let source = match &core_dir {
            Some(path) => format!("the Autoxide checkout at {}", path.display()),
            None => format!("Autoxide by the git tag v{version}"),
        };
        Ok((written.len(), source))
    });
    match result {
        Ok((files, source)) => {
            println!(
                "Created {dir} ({files} files): module `{module}` and the server `acme`, from \
                 examples/external-module."
            );
            println!("It depends on {source}.");
            println!();
            println!("Next (README.md of the new folder, docs/guides/writing-a-module.md):");
            println!(
                "  1. cd {dir} && git init && ./check.sh      (format, lints, tests on SQLite)"
            );
            println!(
                "  2. cargo run -p acme-server -- migrate up && cargo run -p acme-server -- serve"
            );
            println!(
                "  3. Rewrite the tables, service, descriptions and locales/vi.json for your feature;"
            );
            println!(
                "     rename `acme` to your own name. AGENTS.md holds the rules for AI agents."
            );
            println!("  4. cargo xtask check-module {dir}/{module}   (from this repository)");
            println!("  5. Push: .github/workflows/ci.yml runs check.sh on SQLite and PostgreSQL.");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("xtask new-module: {error}");
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
                "       Cargo.toml:            autoxide-{module} = {{ path = \"modules/{module}\", version = \"=<workspace version>\" }}"
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

fn coverage_api(run_tests: bool) -> ExitCode {
    let result = workspace::load()
        .map_err(|error| error.to_string())
        .and_then(|workspace| coverage::run(&workspace.root, run_tests));
    match result {
        Ok(report) => {
            coverage::print(&report);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("xtask coverage-api: {error}");
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
