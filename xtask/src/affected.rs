//! `cargo xtask test-affected [--base <ref>]`: chạy test của các crate bị sửa và của mọi crate
//! phụ thuộc vào chúng, trên SQLite. Đây là phần test của gate nhanh (`./gate.sh --quick`, D-173);
//! gate đầy đủ vẫn chạy toàn bộ test trên cả hai database.
//!
//! "Bị sửa" so với đâu (D-299): mặc định so với **cây file của lần chạy xanh gần nhất** (ref cục
//! bộ `refs/autoxide/quick-green`, không bao giờ được push), chưa có thì so với `origin/main`.
//! Trước đây luôn so với `main`: một nhánh đã sửa `kernel` ở commit đầu thì mọi lần gate nhanh về
//! sau chạy lại test của mọi crate, dù commit đó chỉ sửa một module. So với lần xanh gần nhất vẫn
//! đúng: crate không bị chạm có mã nguồn và phụ thuộc y hệt lúc nó đã xanh. `--base <ref>` giữ
//! cách cũ (so với chỗ rẽ nhánh với `<ref>`).
//!
//! Vì sao không dùng `cargo test -p <crate>`: chỉ chọn vài crate thì cargo hợp feature của thư
//! viện theo riêng các crate đó, khác lúc build cả workspace, nên phải build lại sqlx, sea-orm…
//! thành bộ file thứ hai (đo được 87 s chỉ cho `content`). Cách gom feature cả workspace
//! (`resolver.feature-unification`) còn là tính năng nightly. Vì vậy ở đây build cả workspace —
//! dùng chung cache với gate đầy đủ, cargo chỉ build lại phần đã sửa — rồi chỉ chạy các file
//! test của crate bị ảnh hưởng, giống cách `cargo test` chạy chúng: thư mục làm việc là thư mục
//! crate, có `CARGO_MANIFEST_DIR`. Các file test chạy **song song** (D-299): `cargo test` chạy
//! chúng lần lượt, nên một file test chậm làm cả máy ngồi chờ.
//!
//! Chạy test: khi là gate nhanh trên SQLite (không `--keep-env`, không `--shard`) và có `cargo-nextest`
//! (từ 0.9.85; cài mà cũ hơn thì báo lỗi, không lặng lẽ rơi về bộ chạy cũ) thì cả bước build lẫn chạy giao
//! cho `cargo nextest run --filterset …` — cùng bộ test và cùng cách chọn crate, nhưng mỗi test một tiến
//! trình, nhanh hơn nhiều với chương trình test lớn (`autoxide`: 674 s → 203 s, D-20261125-ci-cancelled-not-failed).
//! Không có nextest thì dùng bộ chạy nêu ở trên. Gate Postgres (`--keep-env`), `--shard` và CI (archive của
//! nextest do `gate.sh` dựng) không đổi.
//!
//! Doc-test không chạy ở đây (cần `rustdoc`); gate đầy đủ chạy chúng.

use std::collections::{BTreeSet, VecDeque};
use std::num::NonZero;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::workspace::Workspace;

/// File ở gốc repo mà đổi thì mọi crate có thể build hoặc chạy khác đi.
const GLOBAL_FILES: &[&str] =
    &["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "clippy.toml", "rustfmt.toml"];
const GLOBAL_DIRS: &[&str] = &[".cargo/"];

/// File nằm ngoài thư mục crate nhưng có test của một crate đọc lúc chạy (bản sinh từ code phải
/// khớp bản đã commit; script mà test so với bản Rust): đổi file thì chạy test của crate đó.
const CHECKED_FILES: &[(&str, &str)] = &[
    ("docs/openapi.json", "autoxide"),
    (".env.example", "autoxide"),
    // `rust_free` của xtask chạy script và đọc danh sách (D-20261125-ci-skip-rust).
    ("scripts/rust-changed.sh", "xtask"),
    ("scripts/rust-free-paths.txt", "xtask"),
];

/// Thư mục ngoài crate có test đọc lúc chạy (`read_dir`, `read_to_string`), như trên. File nhúng lúc
/// biên dịch (`include_str!` / `include_bytes!`) thì không cần ghi ở đây: [`included_files`] tự dò.
/// - `docs/guides/`: `e2e::guides` của `autoxide` chạy mọi ví dụ của hướng dẫn và so khối code
///   bản tiếng Việt với bản tiếng Anh;
/// - `examples/`: test của `xtask` sinh khung module ngoài từ `examples/external-module`.
const CHECKED_DIRS: &[(&str, &str)] = &[("docs/guides/", "autoxide"), ("examples/", "xtask")];

#[derive(Debug, PartialEq, Eq)]
pub enum Affected {
    /// Đổi thứ dùng chung (manifest gốc, `Cargo.lock`, toolchain…): chạy mọi test.
    All,
    /// Tên các crate cần chạy test; rỗng khi chỉ sửa tài liệu.
    Crates(BTreeSet<String>),
}

/// Crate bị ảnh hưởng bởi các file `changed` (đường dẫn tương đối so với gốc repo, dùng `/`).
pub fn affected(workspace: &Workspace, changed: &[String]) -> Affected {
    let mut included = included_files(workspace);
    included.extend(symlinked_files(workspace));
    let mut crates = BTreeSet::new();
    for file in changed {
        if GLOBAL_FILES.contains(&file.as_str())
            || GLOBAL_DIRS.iter().any(|dir| file.starts_with(dir))
        {
            return Affected::All;
        }
        // Crate có test đọc hay nhúng file này dù file nằm ngoài thư mục của nó.
        let readers = CHECKED_FILES
            .iter()
            .filter(|(path, _)| path == file)
            .chain(CHECKED_DIRS.iter().filter(|(dir, _)| file.starts_with(dir)))
            .map(|(_, package)| (*package).to_owned())
            .chain(
                included
                    .iter()
                    .filter(|(path, _)| file == path || file.starts_with(&format!("{path}/")))
                    .map(|(_, package)| package.clone()),
            );
        crates.extend(readers);
        // Crate có thư mục dài nhất chứa file; file ngoài mọi crate (tài liệu…) không kéo
        // theo test nào.
        let owner = workspace
            .packages
            .iter()
            .filter(|package| {
                !package.dir.is_empty() && file.starts_with(&format!("{}/", package.dir))
            })
            .max_by_key(|package| package.dir.len());
        if let Some(package) = owner {
            crates.insert(package.name.clone());
        }
    }
    // Thêm crate phụ thuộc, lặp tới khi không thêm được nữa. Tính mọi loại dependency: crate
    // dùng `testkit` ở `[dev-dependencies]` vẫn phải chạy lại test khi `testkit` đổi.
    loop {
        let before = crates.len();
        for package in &workspace.packages {
            if !crates.contains(&package.name)
                && package.deps.iter().any(|dep| crates.contains(&dep.name))
            {
                crates.insert(package.name.clone());
            }
        }
        if crates.len() == before {
            return Affected::Crates(crates);
        }
    }
}

/// File nằm NGOÀI thư mục crate mà mã nguồn của crate nhúng lúc biên dịch (`include_str!("…")`,
/// `include_bytes!("…")` với đường dẫn tương đối): (đường dẫn so với gốc repo, tên crate). Đổi file
/// đó là đổi crate — vd `docs/MODULES.md` mà `e2e::module_selection` của `autoxide` so với code.
/// Không đọc được repo (test với workspace giả) thì rỗng.
fn included_files(workspace: &Workspace) -> Vec<(String, String)> {
    let Ok(files) = crate::lengths::rust_files(&workspace.root) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for file in files {
        let owner = workspace
            .packages
            .iter()
            .filter(|package| {
                !package.dir.is_empty() && file.starts_with(&format!("{}/", package.dir))
            })
            .max_by_key(|package| package.dir.len());
        let Some(owner) = owner else { continue };
        let Ok(text) = std::fs::read_to_string(workspace.root.join(&file)) else { continue };
        let dir = Path::new(&file).parent().unwrap_or(Path::new(""));
        for literal in included_paths(&text) {
            if let Some(path) = normalize(&dir.join(literal))
                && !path.starts_with(&format!("{}/", owner.dir))
            {
                found.push((path, owner.name.clone()));
            }
        }
    }
    found
}

/// File (hay thư mục) NGOÀI thư mục crate mà một symlink tracked trong thư mục crate trỏ tới: (đích, so với gốc repo;
/// tên crate). Vd `crates/kernel/CHANGELOG.md` → `CHANGELOG.md` gốc, mà kernel nhúng bằng
/// `include_str!("../../CHANGELOG.md")` — đường dẫn trong mã nằm trong crate, nên [`included_files`] không thấy file
/// gốc. Không đọc được repo hay git thì rỗng.
fn symlinked_files(workspace: &Workspace) -> Vec<(String, String)> {
    let Ok(listing) = git(&workspace.root, &["ls-files", "-s"]) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for line in listing.lines() {
        // `<mode> <object> <stage>\t<path>`; 120000 là symlink.
        let Some((meta, link)) = line.split_once('\t') else { continue };
        if !meta.starts_with("120000 ") {
            continue;
        }
        let owner = workspace
            .packages
            .iter()
            .filter(|package| {
                !package.dir.is_empty() && link.starts_with(&format!("{}/", package.dir))
            })
            .max_by_key(|package| package.dir.len());
        let Some(owner) = owner else { continue };
        let Ok(target) = std::fs::read_link(workspace.root.join(link)) else { continue };
        let dir = Path::new(link).parent().unwrap_or(Path::new(""));
        if let Some(path) = normalize(&dir.join(target))
            && !path.starts_with(&format!("{}/", owner.dir))
        {
            found.push((path, owner.name.clone()));
        }
    }
    found
}

/// Đường dẫn trong `include_str!("…")` / `include_bytes!("…")` của một file nguồn.
fn included_paths(text: &str) -> Vec<&str> {
    let mut paths = Vec::new();
    for marker in ["include_str!(\"", "include_bytes!(\""] {
        let mut rest = text;
        while let Some(start) = rest.find(marker) {
            rest = &rest[start + marker.len()..];
            if let Some(end) = rest.find('"') {
                paths.push(&rest[..end]);
            }
        }
    }
    paths
}

/// Bỏ `.` và `..` của một đường dẫn tương đối so với gốc repo, viết bằng `/`; `None` khi nó ra
/// ngoài gốc.
fn normalize(path: &Path) -> Option<String> {
    use std::path::Component;
    let mut parts: Vec<String> = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::CurDir => {}
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(parts.join("/"))
}

/// `cargo xtask test-filter --base <ref>`: filterset của nextest chọn đúng các crate mà
/// `test-affected --base <ref>` chạy — CI build test một lần thành archive của nextest rồi chạy ở
/// nhiều máy, nên việc chọn crate làm trước, ở máy build (D-20261120-ci-nextest-archive).
pub fn filter(workspace: &Workspace, base: &str) -> Result<String, String> {
    let changed = changed_files(&workspace.root, base)?;
    Ok(filterset(&affected(workspace, &changed)))
}

/// Filterset của nextest cho một lựa chọn crate: `all()`, `none()` hay `package(=a) | package(=b)`.
fn filterset(selected: &Affected) -> String {
    match selected {
        Affected::All => "all()".to_owned(),
        Affected::Crates(crates) if crates.is_empty() => "none()".to_owned(),
        Affected::Crates(crates) => {
            crates.iter().map(|name| format!("package(={name})")).collect::<Vec<_>>().join(" | ")
        }
    }
}

/// Cây file của lần `test-affected` xanh gần nhất. Ref cục bộ: `git push` chỉ đẩy nhánh và tag.
const GREEN_REF: &str = "refs/autoxide/quick-green";

/// Mốc để so: chuỗi mô tả cho người đọc và danh sách file đã đổi.
fn changed_since(
    root: &Path,
    base: Option<&str>,
    current: &str,
) -> Result<(String, Vec<String>), String> {
    if let Some(base) = base {
        return Ok((base.to_owned(), changed_files(root, base)?));
    }
    if let Ok(green) = git(root, &["rev-parse", "--verify", "--quiet", GREEN_REF]) {
        let files = git(root, &["diff", "--name-only", "--no-renames", green.trim(), current])?
            .lines()
            .map(str::to_owned)
            .collect();
        return Ok(("the last green run".to_owned(), files));
    }
    let base = default_base(root);
    Ok((base.clone(), changed_files(root, &base)?))
}

/// `keep_env`: chạy trên database và S3 của môi trường (`TEST_DATABASE_URL`, `TEST_S3_ENDPOINT`)
/// như `test-all`, thay vì luôn SQLite — gate của pull request trên CI (`./gate.sh --pr`, D-20261118-ci-pr-affected-tests).
/// Khi đó không ghi mốc "lần xanh gần nhất": mốc đó chỉ dành cho gate nhanh trên SQLite.
pub fn run(
    workspace: &Workspace,
    base: Option<&str>,
    keep_env: bool,
    shard: Shard,
) -> Result<(), String> {
    // Chụp cây file TRƯỚC khi build: xanh thì đúng cây này được ghi là đã xanh.
    let current = snapshot_tree(&workspace.root)?;
    let (base, changed) = changed_since(&workspace.root, base, &current)?;
    let selected = affected(workspace, &changed);
    let dirs: Vec<(String, PathBuf)> = match &selected {
        Affected::All => {
            println!("test-affected: shared build files changed since {base}, running every crate");
            workspace
                .packages
                .iter()
                .map(|package| (package.name.clone(), workspace.root.join(&package.dir)))
                .collect()
        }
        Affected::Crates(crates) if crates.is_empty() => {
            println!("test-affected: no crate changed since {base}, nothing to test");
            if !keep_env && shard.is_whole() {
                git(&workspace.root, &["update-ref", GREEN_REF, &current])?;
            }
            return Ok(());
        }
        Affected::Crates(crates) => {
            println!(
                "test-affected: {} crate(s) changed since {base} or depend on one: {}",
                crates.len(),
                crates.iter().cloned().collect::<Vec<_>>().join(", ")
            );
            workspace
                .packages
                .iter()
                .filter(|package| crates.contains(&package.name))
                .map(|package| (package.name.clone(), workspace.root.join(&package.dir)))
                .collect()
        }
    };

    // Chỉ crate công cụ bị ảnh hưởng (xtask: không phụ thuộc kernel hay module nào, thư viện ngoài
    // chỉ jiff/serde/serde_json) thì dựng riêng chúng: hợp feature khác với cả workspace chỉ làm
    // build lại vài crate nhỏ, còn dựng cả workspace trên máy lạnh mất ~13 phút cho một chương
    // trình test chạy dưới một giây. Có crate khác bị ảnh hưởng thì giữ cách dựng cả workspace.
    let only = match &selected {
        Affected::Crates(crates) => tools_only(workspace, crates),
        Affected::All => None,
    };
    // Gate nhanh (SQLite, chạy cả bộ): chạy bằng nextest nếu có. Đo trên máy 4 nhân với chương trình test của
    // `autoxide` (217 test): libtest 674 s, nextest 203 s — nextest mỗi test một tiến trình nên không dính chung
    // trạng thái của cả chương trình, và chia việc theo từng test thay vì theo từng chương trình. Cùng bộ test
    // với đường dưới (CI cũng chạy bằng nextest, D-20261120-ci-nextest-archive); không có nextest thì dùng đường dưới.
    if !keep_env && shard.is_whole() && nextest_available()? {
        let started = Instant::now();
        run_nextest(&workspace.root, &selected, only.as_deref())?;
        git(&workspace.root, &["update-ref", GREEN_REF, &current])?;
        println!("\ntest-affected: all tests passed (nextest) in {:.0?}", started.elapsed());
        return Ok(());
    }
    let jobs: Vec<Job> = build_tests(&workspace.root, only.as_deref())?
        .into_iter()
        .filter_map(|test| {
            let (name, dir) = dirs.iter().find(|(_, dir)| *dir == test.dir)?;
            Some(Job {
                name: name.clone(),
                dir: dir.clone(),
                executable: test.executable,
                threads: 0,
                tests: None,
            })
        })
        .collect();
    let jobs = shard.apply(jobs)?;
    let workers = workers();
    println!("test-affected: running {} test binaries, {workers} at a time", jobs.len());
    let started = Instant::now();
    let mode = if keep_env { Mode::from_env() } else { Mode::SqliteOnly };
    let mut jobs = jobs;
    if matches!(mode, Mode::Postgres) {
        share_postgres(&mut jobs)?;
    }
    let failed = run_parallel(&workspace.root, jobs, workers, &mode)?;
    if failed.is_empty() {
        // Mốc "lần xanh gần nhất" chỉ ghi khi đã chạy hết, không chỉ một phần.
        if !keep_env && shard.is_whole() {
            git(&workspace.root, &["update-ref", GREEN_REF, &current])?;
        }
        println!("\ntest-affected: all test binaries passed in {:.0?}", started.elapsed());
        Ok(())
    } else {
        Err(format!("{} test binaries failed: {}", failed.len(), failed.join(", ")))
    }
}

/// `cargo xtask test-all`: mọi test của workspace trên database của `TEST_DATABASE_URL` (trống là
/// SQLite), các file test chạy song song — phần test của gate đầy đủ (D-299). `--doc` chạy thêm
/// doc-test (gate chạy một lần, ở lượt SQLite; chia phần thì chỉ phần đầu chạy).
pub fn run_all(workspace: &Workspace, doc: bool, shard: Shard) -> Result<(), String> {
    let workers = workers();
    let mode = Mode::from_env();
    let jobs: Vec<Job> = build_tests(&workspace.root, None)?
        .into_iter()
        .filter_map(|test| {
            let package = workspace
                .packages
                .iter()
                .find(|package| workspace.root.join(&package.dir) == test.dir)?;
            Some(Job {
                name: package.name.clone(),
                dir: test.dir,
                executable: test.executable,
                threads: 0,
                tests: None,
            })
        })
        .collect();
    let mut jobs = shard.apply(jobs)?;
    if matches!(mode, Mode::Postgres) {
        share_postgres(&mut jobs)?;
    }
    println!("test-all: running {} test binaries, {workers} at a time", jobs.len());
    let started = Instant::now();
    let failed = run_parallel(&workspace.root, jobs, workers, &mode)?;
    if !failed.is_empty() {
        return Err(format!("{} test binaries failed: {}", failed.len(), failed.join(", ")));
    }
    println!("\ntest-all: all test binaries passed in {:.0?}", started.elapsed());
    if doc && shard.index == 1 {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
        let status = without_cargo_run_env(&mut Command::new(&cargo))
            .args(["test", "--workspace", "--locked", "--doc"])
            .current_dir(&workspace.root)
            .status()
            .map_err(|error| format!("cannot run `{cargo} test --doc`: {error}"))?;
        if !status.success() {
            return Err("doc tests failed (see above)".to_owned());
        }
    }
    Ok(())
}

/// Số test chạy cùng lúc trên Postgres, cộng mọi chương trình test. Xem [`share_postgres`].
const POSTGRES_CONCURRENT_TESTS: usize = 16;

/// Cách chạy một chương trình test.
enum Mode {
    /// Gate nhanh: luôn SQLite, bỏ biến database và S3 người chạy lỡ đặt.
    SqliteOnly,
    /// Giữ nguyên môi trường (gate đầy đủ, lượt SQLite).
    Inherit,
    /// Giữ nguyên môi trường; mỗi chương trình chỉ chạy số test cùng lúc đã chia (`Job::threads`).
    Postgres,
}

impl Mode {
    /// Theo database của `TEST_DATABASE_URL`.
    fn from_env() -> Mode {
        let postgres = std::env::var("TEST_DATABASE_URL")
            .is_ok_and(|url| url.starts_with("postgres:") || url.starts_with("postgresql:"));
        if postgres { Mode::Postgres } else { Mode::Inherit }
    }
}

/// Chia [`POSTGRES_CONCURRENT_TESTS`] suất cho các chương trình test theo số test của từng chương
/// trình (D-20261118-build-size). Mỗi test giữ tới vài kết nối; đo được 16 test cùng lúc ≈ 141 kết nối đỉnh, dưới
/// trần 300 của gate. Trước đây mỗi file test là một chương trình và suất chia đều; nay test tích
/// hợp của một crate gộp thành một chương trình hàng trăm test, chia đều thì nó chỉ được một hai
/// suất và chạy gần như tuần tự. [`run_parallel`] chỉ cho chạy cùng lúc các chương trình có tổng
/// suất không vượt trần, nên số test chạy cùng lúc vẫn không vượt trần.
fn share_postgres(jobs: &mut [Job]) -> Result<(), String> {
    let counts: Vec<usize> = jobs.iter().map(count_tests).collect::<Result<_, _>>()?;
    let total: usize = counts.iter().sum();
    for (job, count) in jobs.iter_mut().zip(counts) {
        job.threads = postgres_threads(count, total);
    }
    // Chương trình lớn chạy trước: nó quyết định tổng thời gian.
    jobs.sort_by_key(|job| std::cmp::Reverse(job.threads));
    Ok(())
}

/// Suất ít nhất của một chương trình test trên Postgres (hoặc số test của nó, nếu ít hơn).
///
/// Chia theo số test thì chương trình ít test mà mỗi test nặng chỉ được một suất và chạy tuần tự:
/// test đầu-cuối của `autoxide` (125 test, mỗi test dựng mọi module) được 1/16 suất, chạy 10 phút
/// rưỡi, sáu phút cuối chạy một mình khi các chương trình khác đã xong (D-20261119-ci-faster).
const POSTGRES_MIN_THREADS: usize = 4;

/// Số suất của một chương trình có `count` test, trong tổng `total` test: theo tỉ lệ, nhưng không
/// dưới [`POSTGRES_MIN_THREADS`] (trừ khi nó có ít test hơn) và không quá trần.
fn postgres_threads(count: usize, total: usize) -> usize {
    let share = (POSTGRES_CONCURRENT_TESTS * count).div_ceil(total.max(1));
    let floor = count.min(POSTGRES_MIN_THREADS);
    share.max(floor).clamp(1, POSTGRES_CONCURRENT_TESTS)
}

/// Số test của một chương trình test (`--list`, không chạy gì).
fn count_tests(job: &Job) -> Result<usize, String> {
    if let Some(tests) = &job.tests {
        return Ok(tests.len());
    }
    Ok(list_tests(job)?.len())
}

/// Tên mọi test của một chương trình test (`--list`, không chạy gì), theo thứ tự nó in ra.
fn list_tests(job: &Job) -> Result<Vec<String>, String> {
    let output = without_cargo_run_env(&mut Command::new(&job.executable))
        .args(["--list", "--format", "terse"])
        .current_dir(&job.dir)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| {
            format!("cannot list the tests of {}: {error}", job.executable.display())
        })?;
    if !output.status.success() {
        return Err(format!("cannot list the tests of {}", job.executable.display()));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.strip_suffix(": test"))
        .map(str::to_owned)
        .collect())
}

/// Phần thứ `index` (từ 1) trong `count` phần của mọi test: CI chạy các phần trên nhiều máy cùng lúc
/// (`./gate.sh --shard`, D-20261119-ci-faster). Mỗi chương trình test chia đều theo kiểu chia bài
/// (test thứ i vào phần i mod n), nên chương trình nặng cũng được chia ra, không dồn vào một máy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shard {
    pub index: usize,
    pub count: usize,
}

impl Shard {
    /// Không chia: chạy mọi test.
    pub const WHOLE: Shard = Shard { index: 1, count: 1 };

    /// Đọc `k/n` với 1 ≤ k ≤ n.
    pub fn parse(text: &str) -> Option<Shard> {
        let (index, count) = text.split_once('/')?;
        let (index, count) = (index.parse().ok()?, count.parse().ok()?);
        (1..=count).contains(&index).then_some(Shard { index, count })
    }

    fn is_whole(self) -> bool {
        self.count == 1
    }

    /// Test thứ `position` (từ 0) của một chương trình có thuộc phần này không.
    fn holds(self, position: usize) -> bool {
        position % self.count == self.index - 1
    }

    /// Giữ phần test của shard này trong mỗi chương trình; bỏ chương trình không còn test nào.
    fn apply(self, jobs: Vec<Job>) -> Result<Vec<Job>, String> {
        if self.is_whole() {
            return Ok(jobs);
        }
        let mut kept = Vec::new();
        for mut job in jobs {
            let tests: Vec<String> = list_tests(&job)?
                .into_iter()
                .enumerate()
                .filter(|(position, _)| self.holds(*position))
                .map(|(_, name)| name)
                .collect();
            if !tests.is_empty() {
                job.tests = Some(tests);
                kept.push(job);
            }
        }
        println!(
            "shard {}/{}: {} test binaries have tests in this part",
            self.index,
            self.count,
            kept.len()
        );
        Ok(kept)
    }
}

fn workers() -> usize {
    std::thread::available_parallelism().map_or(1, NonZero::get).min(MAX_WORKERS)
}

/// Trần số file test chạy cùng lúc. Mỗi file test tự chạy các test của nó trên nhiều thread,
/// nên không cần nhiều hơn số nhân; trần giữ RAM vừa phải trên máy nhiều nhân.
const MAX_WORKERS: usize = 8;

struct Job {
    name: String,
    dir: PathBuf,
    executable: PathBuf,
    /// Trên Postgres: số test chạy cùng lúc của chương trình này (`--test-threads`); 0: mặc định.
    threads: usize,
    /// Chỉ chạy các test này (`--shard`); `None`: mọi test.
    tests: Option<Vec<String>>,
}

/// Chạy các file test, `workers` file cùng lúc; trả tên các file đỏ.
///
/// Output của từng file được giữ lại và chỉ in khi file đó đỏ, để log không trộn lẫn nhau.
fn run_parallel(
    root: &Path,
    jobs: Vec<Job>,
    workers: usize,
    mode: &Mode,
) -> Result<Vec<String>, String> {
    // Hàng đợi chung kèm tổng suất Postgres đang dùng; `Condvar` đánh thức thread đang chờ suất.
    let queue = Mutex::new(Queue { jobs: jobs.into(), used: 0 });
    let freed = Condvar::new();
    let failed = Mutex::new(Vec::new());
    let errors = Mutex::new(Vec::new());
    // `thread::scope`: các thread mượn được `queue`, `failed`… vì scope chờ chúng xong mới
    // trả về — Rust biết các biến sống lâu hơn thread, không cần `Arc`.
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                // Guard của `lock()` bị huỷ ngay trong closure: lấy việc xong là nhả hàng đợi,
                // không giữ nó suốt lúc chạy test.
                while let Some(job) = next_job(&queue, &freed) {
                    let display = job
                        .executable
                        .strip_prefix(root)
                        .unwrap_or(&job.executable)
                        .display()
                        .to_string();
                    let started = Instant::now();
                    match run_one(&job, mode) {
                        Ok(output) if output.status.success() => {
                            println!(
                                "     ok {display} ({}) {}",
                                job.name,
                                seconds(started.elapsed())
                            );
                        }
                        Ok(output) => {
                            // Cả khối in một lần, giữ lock của stdout cho khỏi xen dòng file khác.
                            let mut text = format!(
                                "\n---- FAILED {display} ({}) {} ----\n",
                                job.name,
                                seconds(started.elapsed())
                            );
                            text.push_str(&String::from_utf8_lossy(&output.stdout));
                            text.push_str(&String::from_utf8_lossy(&output.stderr));
                            println!("{text}");
                            if let Ok(mut failed) = failed.lock() {
                                failed.push(display);
                            }
                        }
                        Err(error) => {
                            if let Ok(mut errors) = errors.lock() {
                                errors.push(format!("cannot run {display}: {error}"));
                            }
                        }
                    }
                    if let Ok(mut queue) = queue.lock() {
                        queue.used -= job.threads;
                    }
                    freed.notify_all();
                }
            });
        }
    });
    let errors = errors.into_inner().unwrap_or_default();
    if !errors.is_empty() {
        return Err(errors.join("; "));
    }
    Ok(failed.into_inner().unwrap_or_default())
}

struct Queue {
    jobs: VecDeque<Job>,
    /// Tổng `threads` của các chương trình đang chạy.
    used: usize,
}

/// Lấy chương trình kế tiếp còn vừa suất (đang không chạy gì thì luôn vừa); hết việc thì `None`.
fn next_job(queue: &Mutex<Queue>, freed: &Condvar) -> Option<Job> {
    let mut queue = queue.lock().ok()?;
    loop {
        if queue.jobs.is_empty() {
            return None;
        }
        let used = queue.used;
        let fits = |job: &Job| used == 0 || used + job.threads <= POSTGRES_CONCURRENT_TESTS;
        if let Some(index) = queue.jobs.iter().position(fits) {
            let job = queue.jobs.remove(index)?;
            queue.used += job.threads;
            return Some(job);
        }
        // `wait` nhả lock trong lúc chờ và lấy lại khi được đánh thức.
        queue = freed.wait(queue).ok()?;
    }
}

fn run_one(job: &Job, mode: &Mode) -> std::io::Result<Output> {
    let mut command = Command::new(&job.executable);
    without_cargo_run_env(&mut command)
        .current_dir(&job.dir)
        .env("CARGO_MANIFEST_DIR", &job.dir)
        .env("CARGO_PKG_NAME", &job.name)
        .stdin(Stdio::null());
    match mode {
        Mode::SqliteOnly => {
            command.env_remove("TEST_DATABASE_URL").env_remove("TEST_S3_ENDPOINT");
        }
        Mode::Inherit => {}
        Mode::Postgres => {
            command.arg(format!("--test-threads={}", job.threads.max(1)));
        }
    }
    if let Some(tests) = &job.tests {
        // Nhiều tên sau `--exact`: chạy đúng các test có tên đó.
        command.arg("--exact").args(tests);
    }
    command.output()
}

fn seconds(elapsed: Duration) -> String {
    format!("{:.1}s", elapsed.as_secs_f64())
}

/// Cây file (git tree) của thư mục làm việc như nó đang có: file đã commit, đã sửa, file mới
/// (trừ file bị `.gitignore`), không tính file đã xoá. Dùng một index tạm riêng nên không đụng
/// tới phần người dùng đã `git add`.
fn snapshot_tree(root: &Path) -> Result<String, String> {
    let git_dir = git(root, &["rev-parse", "--absolute-git-dir"])?;
    let index = PathBuf::from(git_dir.trim()).join("autoxide-test-affected.index");
    let index = index.to_str().ok_or("the git directory path is not UTF-8")?.to_owned();
    let with_index = |args: &[&str]| -> Result<String, String> {
        git_with(root, args, &[("GIT_INDEX_FILE", index.as_str())])
    };
    with_index(&["read-tree", "HEAD"])?;
    with_index(&["add", "--all", "--", "."])?;
    let tree = with_index(&["write-tree"])?;
    Ok(tree.trim().to_owned())
}

/// Mặc định so với `origin/main` (bản đã lên GitHub), không có thì `main`.
fn default_base(root: &Path) -> String {
    let remote = git(root, &["rev-parse", "--verify", "--quiet", "origin/main"]);
    if remote.is_ok() { "origin/main".to_owned() } else { "main".to_owned() }
}

/// File đã đổi kể từ chỗ rẽ nhánh với `base`: đã commit, đã stage, chưa stage và file mới.
fn changed_files(root: &Path, base: &str) -> Result<Vec<String>, String> {
    let merge_base = git(root, &["merge-base", "HEAD", base])?;
    let mut files: Vec<String> = git(root, &["diff", "--name-only", merge_base.trim()])?
        .lines()
        .map(str::to_owned)
        .collect();
    files.extend(
        git(root, &["ls-files", "--others", "--exclude-standard"])?.lines().map(str::to_owned),
    );
    Ok(files)
}

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    git_with(root, args, &[])
}

fn git_with(root: &Path, args: &[&str], env: &[(&str, &str)]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .envs(env.iter().copied())
        .current_dir(root)
        .output()
        .map_err(|error| format!("cannot run git: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "`git {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|error| format!("git output is not UTF-8: {error}"))
}

/// Bỏ các biến `cargo run` đặt cho chính xtask (`CARGO_MANIFEST_DIR`, `CARGO_PKG_*`…).
///
/// Vì sao: build script của vài thư viện (vd `ring`) khai `rerun-if-env-changed` cho các biến
/// này, và cargo so giá trị trong môi trường của chính nó. Để nguyên thì cargo gọi từ xtask
/// thấy chúng khác với cargo gọi thẳng từ `gate.sh`, nên build lại `ring` cùng mọi thứ phụ
/// thuộc vào nó (khoảng 36 crate) mỗi lần đổi qua lại.
fn without_cargo_run_env(command: &mut Command) -> &mut Command {
    for (name, _) in std::env::vars_os() {
        let Some(name) = name.to_str() else { continue };
        if name.starts_with("CARGO_PKG_")
            || name.starts_with("CARGO_MANIFEST_")
            || matches!(
                name,
                "CARGO_CRATE_NAME" | "CARGO_BIN_NAME" | "CARGO_PRIMARY_PACKAGE" | "OUT_DIR"
            )
        {
            command.env_remove(name);
        }
    }
    command
}

struct TestBinary {
    executable: PathBuf,
    /// Thư mục crate chứa test này.
    dir: PathBuf,
}

// Đúng những trường cần của từng dòng JSON mà `cargo --message-format=json` in ra.
#[derive(Deserialize)]
struct Message {
    reason: String,
    manifest_path: Option<PathBuf>,
    executable: Option<PathBuf>,
    profile: Option<Profile>,
}

#[derive(Deserialize)]
struct Profile {
    test: bool,
}

/// Bản nextest thấp nhất hiểu `--filterset` và `--no-tests` (bản ghim ở `scripts/cloud-setup.sh` cao hơn nhiều).
const NEXTEST_MIN: (u64, u64, u64) = (0, 9, 85);

/// `Ok(true)`: có nextest đủ mới; `Ok(false)`: không cài (dùng bộ chạy cũ); `Err`: cài nhưng quá cũ — đỏ,
/// không lặng lẽ rơi về bộ cũ để gate trên máy này khác gate ở chỗ khác.
fn nextest_available() -> Result<bool, String> {
    let Ok(output) = Command::new("cargo-nextest").arg("--version").stdin(Stdio::null()).output()
    else {
        return Ok(false);
    };
    if !output.status.success() {
        return Ok(false);
    }
    let text = String::from_utf8_lossy(&output.stdout);
    match parse_nextest_version(&text) {
        Some(version) if version >= NEXTEST_MIN => Ok(true),
        _ => Err(format!(
            "cargo-nextest is too old or its version is unreadable (`{}`); `test-affected` needs \
             {}.{}.{} or newer — upgrade it (`scripts/cloud-setup.sh` installs the pinned one, or \
             `cargo install cargo-nextest --locked`)",
            text.trim(),
            NEXTEST_MIN.0,
            NEXTEST_MIN.1,
            NEXTEST_MIN.2
        )),
    }
}

/// `cargo-nextest 0.9.152 (…)` → `(0, 9, 152)`.
fn parse_nextest_version(text: &str) -> Option<(u64, u64, u64)> {
    let word =
        text.split_whitespace().find(|word| word.starts_with(|c: char| c.is_ascii_digit()))?;
    let mut parts = word.split(|c: char| !c.is_ascii_digit()).map(|part| part.parse::<u64>().ok());
    Some((parts.next()??, parts.next()??, parts.next()??))
}

/// Build và chạy test của các crate đã chọn bằng nextest, chỉ SQLite (bỏ biến Postgres và S3 của người chạy).
fn run_nextest(root: &Path, selected: &Affected, only: Option<&[String]>) -> Result<(), String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let mut command = Command::new(&cargo);
    without_cargo_run_env(&mut command).args(["nextest", "run", "--locked"]);
    match only {
        Some(names) => {
            command.args(names.iter().flat_map(|name| ["-p", name.as_str()]));
        }
        None => {
            command.arg("--workspace");
        }
    }
    // Mỗi nhân hai test: nhiều test chờ I/O (cùng cách `./gate.sh` chạy nextest).
    let threads = std::thread::available_parallelism().map_or(2, NonZero::get) * 2;
    command
        .args(["--filterset", &filterset(selected)])
        .args(["--no-fail-fast", "--no-tests", "pass", "--test-threads", &threads.to_string()])
        .env_remove("TEST_DATABASE_URL")
        .env_remove("TEST_S3_ENDPOINT")
        .current_dir(root)
        .stdin(Stdio::null());
    let status =
        command.status().map_err(|error| format!("cannot run `{cargo} nextest`: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("nextest reported failing tests (see above)".to_owned())
    }
}

/// Tên các crate được chọn nếu TẤT CẢ đều là crate công cụ (`Role::Tool`); không thì `None`.
fn tools_only(workspace: &Workspace, crates: &BTreeSet<String>) -> Option<Vec<String>> {
    let selected: Vec<&crate::workspace::Package> =
        workspace.packages.iter().filter(|package| crates.contains(&package.name)).collect();
    let all_tools = !selected.is_empty()
        && selected.iter().all(|package| matches!(package.role, crate::workspace::Role::Tool));
    all_tools.then(|| selected.iter().map(|package| package.name.clone()).collect())
}

/// Build file test của workspace (như gate đầy đủ, để dùng chung cache) hay chỉ của `only` (xem
/// [`tools_only`]) và trả danh sách.
fn build_tests(root: &Path, only: Option<&[String]>) -> Result<Vec<TestBinary>, String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let mut scope: Vec<&str> = vec!["--workspace"];
    if let Some(names) = only {
        scope = names.iter().flat_map(|name| ["-p", name.as_str()]).collect();
    }
    let output = without_cargo_run_env(&mut Command::new(&cargo))
        .args(["test"])
        .args(&scope)
        .args([
            "--locked",
            "--no-run",
            // Lỗi biên dịch vẫn in ra stderr như thường; stdout chỉ còn JSON để đọc.
            "--message-format=json-render-diagnostics",
        ])
        .current_dir(root)
        .stderr(Stdio::inherit())
        .output()
        .map_err(|error| format!("cannot run `{cargo} test`: {error}"))?;
    if !output.status.success() {
        return Err("building the tests failed (see the errors above)".to_owned());
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|error| format!("cargo output is not UTF-8: {error}"))?;
    Ok(stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Message>(line).ok())
        .filter(|message| message.reason == "compiler-artifact")
        .filter(|message| message.profile.as_ref().is_some_and(|profile| profile.test))
        .filter_map(|message| {
            let dir = message.manifest_path?.parent()?.to_path_buf();
            Some(TestBinary { executable: message.executable?, dir })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{Dep, DepKind, Package, Role};

    #[test]
    fn suat_postgres_theo_ti_le_nhung_khong_qua_it() {
        // Chương trình nhiều test lấy phần theo tỉ lệ.
        assert_eq!(postgres_threads(1000, 2000), 8);
        // Chương trình ít test mà nặng vẫn được vài suất, không chạy tuần tự.
        assert_eq!(postgres_threads(125, 2000), POSTGRES_MIN_THREADS);
        // Ít test hơn mức sàn: mỗi test một suất là đủ.
        assert_eq!(postgres_threads(2, 2000), 2);
        assert_eq!(postgres_threads(0, 2000), 1);
        // Không vượt trần, kể cả khi chỉ có một chương trình.
        assert_eq!(postgres_threads(500, 500), POSTGRES_CONCURRENT_TESTS);
    }

    #[test]
    fn doc_phien_ban_nextest() {
        assert_eq!(parse_nextest_version("cargo-nextest 0.9.152"), Some((0, 9, 152)));
        assert_eq!(
            parse_nextest_version("cargo-nextest 0.9.152 (abc 2026-01-01)"),
            Some((0, 9, 152))
        );
        assert!(parse_nextest_version("cargo-nextest 0.9.64").is_some_and(|v| v < NEXTEST_MIN));
        assert_eq!(parse_nextest_version("không phải số"), None);
    }

    #[test]
    fn chi_crate_cong_cu_thi_dung_rieng() {
        let ws = workspace();
        let set = |names: &[&str]| names.iter().map(|name| (*name).to_owned()).collect();
        assert_eq!(tools_only(&ws, &set(&["xtask"])), Some(vec!["xtask".to_owned()]));
        assert_eq!(tools_only(&ws, &set(&["xtask", "autoxide-notes"])), None);
        assert_eq!(tools_only(&ws, &set(&["autoxide-kernel"])), None);
        assert_eq!(tools_only(&ws, &set(&[])), None);
    }

    #[test]
    fn shard_doc_dung_va_chia_deu_moi_test() {
        assert_eq!(Shard::parse("2/3"), Some(Shard { index: 2, count: 3 }));
        assert_eq!(Shard::parse("0/2"), None);
        assert_eq!(Shard::parse("3/2"), None);
        assert_eq!(Shard::parse("1/0"), None);
        assert_eq!(Shard::parse("x"), None);
        // Mỗi test thuộc đúng một phần.
        let shards = [
            Shard { index: 1, count: 3 },
            Shard { index: 2, count: 3 },
            Shard { index: 3, count: 3 },
        ];
        for position in 0..10 {
            assert_eq!(shards.iter().filter(|shard| shard.holds(position)).count(), 1);
        }
        assert!(Shard::WHOLE.holds(7));
    }

    fn package(name: &str, dir: &str, deps: &[(&str, DepKind)]) -> Package {
        Package {
            name: name.to_owned(),
            license: None,
            dir: dir.to_owned(),
            role: Role::from_dir(dir),
            deps: deps
                .iter()
                .map(|(name, kind)| Dep { name: (*name).to_owned(), kind: *kind })
                .collect(),
        }
    }

    /// kernel ← testkit (dev) ← notes, content ← autoxide; xtask đứng riêng.
    fn workspace() -> Workspace {
        use DepKind::{Dev, Normal};
        Workspace {
            root: "/repo".into(),
            packages: vec![
                package("autoxide-kernel", "crates/kernel", &[("serde", Normal)]),
                package("autoxide-testkit", "crates/testkit", &[("autoxide-kernel", Normal)]),
                package(
                    "autoxide-notes",
                    "modules/notes",
                    &[("autoxide-kernel", Normal), ("autoxide-testkit", Dev)],
                ),
                package("autoxide-content", "modules/content", &[("autoxide-kernel", Normal)]),
                package(
                    "autoxide",
                    "crates/app",
                    &[("autoxide-notes", Normal), ("autoxide-content", Normal)],
                ),
                package("xtask", "xtask", &[("serde", Normal)]),
            ],
        }
    }

    fn crates(names: &[&str]) -> Affected {
        Affected::Crates(names.iter().map(|name| (*name).to_owned()).collect())
    }

    fn changed(files: &[&str]) -> Affected {
        let files: Vec<String> = files.iter().map(|file| (*file).to_owned()).collect();
        affected(&workspace(), &files)
    }

    #[test]
    fn filterset_cua_nextest_theo_lua_chon() {
        assert_eq!(filterset(&Affected::All), "all()");
        assert_eq!(filterset(&crates(&[])), "none()");
        assert_eq!(
            filterset(&crates(&["autoxide", "autoxide-notes"])),
            "package(=autoxide) | package(=autoxide-notes)"
        );
    }

    #[test]
    fn sua_module_chay_module_va_binary() {
        assert_eq!(
            changed(&["modules/content/src/entries/body.rs"]),
            crates(&["autoxide", "autoxide-content"])
        );
    }

    #[test]
    fn sua_kernel_chay_moi_crate_dua_vao_no() {
        assert_eq!(
            changed(&["crates/kernel/src/lib.rs"]),
            crates(&[
                "autoxide",
                "autoxide-content",
                "autoxide-kernel",
                "autoxide-notes",
                "autoxide-testkit",
            ])
        );
    }

    #[test]
    fn dev_dependency_cung_keo_theo() {
        assert_eq!(
            changed(&["crates/testkit/src/app.rs"]),
            crates(&["autoxide", "autoxide-notes", "autoxide-testkit"])
        );
    }

    #[test]
    fn chi_sua_tai_lieu_thi_khong_chay_gi() {
        assert_eq!(
            changed(&["docs/ROADMAP.md", "README.md", ".claude/skills/x/SKILL.md"]),
            crates(&[])
        );
        assert_eq!(changed(&[]), crates(&[]));
    }

    #[test]
    fn file_dung_chung_thi_chay_het() {
        for file in ["Cargo.lock", "Cargo.toml", "rust-toolchain.toml", ".cargo/config.toml"] {
            assert_eq!(changed(&["docs/ROADMAP.md", file]), Affected::All, "{file}");
        }
        // Manifest của một crate không phải file dùng chung.
        assert_eq!(changed(&["xtask/Cargo.toml"]), crates(&["xtask"]));
    }

    #[test]
    fn huong_dan_va_vi_du_chay_test_doc_chung() {
        // Lô #394: sửa bản tiếng Việt của một hướng dẫn mà không chạy `e2e::guides`, main đỏ ngầm.
        assert_eq!(changed(&["docs/guides/writing-a-module.vi.md"]), crates(&["autoxide"]));
        assert_eq!(changed(&["docs/guides/files/photo.png"]), crates(&["autoxide"]));
        assert_eq!(changed(&["examples/external-module/AGENTS.md"]), crates(&["xtask"]));
    }

    #[test]
    fn duong_dan_nhung_va_chuan_hoa() {
        let text = r#"const A: &str = include_str!("../../../../docs/MODULES.md");
            const B: &[u8] = include_bytes!("../locales/x.bin"); fn c() { include_str!("y.txt"); }"#;
        let mut paths = included_paths(text);
        paths.sort();
        assert_eq!(paths, ["../../../../docs/MODULES.md", "../locales/x.bin", "y.txt"]);
        let dir = Path::new("crates/app/src/e2e");
        assert_eq!(
            normalize(&dir.join("../../../../docs/MODULES.md")).as_deref(),
            Some("docs/MODULES.md")
        );
        assert_eq!(normalize(&dir.join("./a/../b.rs")).as_deref(), Some("crates/app/src/e2e/b.rs"));
        assert_eq!(normalize(Path::new("../outside")), None);
    }

    /// Dò trên một repo thật: file nhúng ngoài crate kéo crate đó, file nhúng trong crate thì không
    /// cần (đã thuộc crate).
    #[test]
    fn file_nhung_ngoai_crate_keo_crate_nhung_no() {
        let repo = TempRepo::new("include");
        repo.write(
            "crates/app/src/e2e/modules.rs",
            "const M: &str = include_str!(\"../../../../docs/MODULES.md\");\n",
        );
        repo.write(
            "crates/app/src/lib.rs",
            "const V: &str = include_str!(\"../locales/vi.json\");\n",
        );
        repo.write("docs/MODULES.md", "x");
        let mut workspace = workspace();
        workspace.root = repo.0.clone();
        assert_eq!(
            included_files(&workspace),
            vec![("docs/MODULES.md".to_owned(), "autoxide".to_owned())]
        );
        let files = vec!["docs/MODULES.md".to_owned()];
        assert_eq!(affected(&workspace, &files), crates(&["autoxide"]));
    }

    #[test]
    fn file_sinh_tu_code_chay_test_cua_binary() {
        assert_eq!(changed(&["docs/openapi.json"]), crates(&["autoxide"]));
        assert_eq!(changed(&[".env.example"]), crates(&["autoxide"]));
    }

    /// Repo git tạm có một commit; tự xoá khi ra khỏi scope.
    struct TempRepo(PathBuf);

    impl TempRepo {
        fn new(name: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir =
                std::env::temp_dir().join(format!("xtask-{name}-{}-{nanos}", std::process::id()));
            std::fs::create_dir_all(dir.join("modules/notes")).unwrap();
            std::fs::write(dir.join(".gitignore"), "target/\n").unwrap();
            std::fs::write(dir.join("modules/notes/a.rs"), "a").unwrap();
            std::fs::write(dir.join("modules/notes/b.rs"), "b").unwrap();
            let repo = TempRepo(dir);
            repo.git(&["init", "--quiet"]);
            repo.git(&["add", "--all"]);
            repo.git(&["commit", "--quiet", "--message", "init"]);
            repo
        }

        fn git(&self, args: &[&str]) -> String {
            let mut all = vec!["-c", "user.name=test", "-c", "user.email=test@example.invalid"];
            all.extend_from_slice(args);
            git(&self.0, &all).unwrap()
        }

        fn write(&self, file: &str, content: &str) {
            std::fs::create_dir_all(self.0.join(file).parent().unwrap()).unwrap();
            std::fs::write(self.0.join(file), content).unwrap();
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn so_voi_lan_xanh_gan_nhat_chi_lay_file_doi_sau_do() {
        let repo = TempRepo::new("green");
        // Chưa có mốc xanh: so với nhánh được nêu, như cách cũ.
        repo.write("modules/notes/a.rs", "a2");
        let tree = snapshot_tree(&repo.0).unwrap();
        let (_, files) = changed_since(&repo.0, Some("HEAD"), &tree).unwrap();
        assert_eq!(files, vec!["modules/notes/a.rs".to_owned()]);

        // Ghi mốc xanh tại cây đó; sau đó sửa b, thêm c, xoá a, thêm file bị .gitignore.
        repo.git(&["update-ref", GREEN_REF, &tree]);
        repo.write("modules/notes/b.rs", "b2");
        repo.write("modules/notes/c.rs", "c");
        repo.write("target/debug/x", "ignored");
        std::fs::remove_file(repo.0.join("modules/notes/a.rs")).unwrap();
        let tree = snapshot_tree(&repo.0).unwrap();
        let (base, mut files) = changed_since(&repo.0, None, &tree).unwrap();
        files.sort();
        assert_eq!(base, "the last green run");
        assert_eq!(files, ["modules/notes/a.rs", "modules/notes/b.rs", "modules/notes/c.rs"]);

        // Cây không đổi thì không có file nào; phần đã `git add` của người dùng giữ nguyên.
        repo.git(&["update-ref", GREEN_REF, &tree]);
        let (_, files) = changed_since(&repo.0, None, &snapshot_tree(&repo.0).unwrap()).unwrap();
        assert!(files.is_empty(), "{files:?}");
        assert_eq!(repo.git(&["diff", "--cached", "--name-only"]), "");
    }

    /// `crates/kernel/CHANGELOG.md` là symlink tới `CHANGELOG.md` gốc mà kernel nhúng: đổi file gốc phải chạy test của
    /// kernel (trước đây `test-filter` bỏ sót). Symlink nằm ngoài crate (`AGENTS.md` → `CLAUDE.md`) thì không kéo gì.
    #[cfg(unix)]
    #[test]
    fn symlink_trong_crate_keo_file_no_tro_toi() {
        let repo = TempRepo::new("symlink");
        repo.write("CHANGELOG.md", "# Changelog");
        repo.write("CLAUDE.md", "rules");
        repo.write("docs/shared/x.json", "{}");
        std::os::unix::fs::symlink("../../CHANGELOG.md", repo.0.join("modules/notes/CHANGELOG.md"))
            .unwrap();
        std::os::unix::fs::symlink("../../docs/shared", repo.0.join("modules/notes/shared"))
            .unwrap();
        std::os::unix::fs::symlink("CLAUDE.md", repo.0.join("AGENTS.md")).unwrap();
        repo.git(&["add", "--all"]);
        let mut workspace = workspace();
        workspace.root = repo.0.clone();
        let changed = |files: &[&str]| {
            affected(&workspace, &files.iter().map(|file| (*file).to_owned()).collect::<Vec<_>>())
        };
        assert_eq!(changed(&["CHANGELOG.md"]), crates(&["autoxide", "autoxide-notes"]));
        assert_eq!(changed(&["docs/shared/x.json"]), crates(&["autoxide", "autoxide-notes"]));
        assert_eq!(changed(&["CLAUDE.md"]), crates(&[]));
        assert_eq!(changed(&["docs/shared-other.json"]), crates(&[]));
    }

    #[test]
    fn thu_muc_trung_tien_to_khong_bi_nham() {
        // `modules/notes-extra` không thuộc `modules/notes`.
        assert_eq!(changed(&["modules/notes-extra/README.md"]), crates(&[]));
    }
}
