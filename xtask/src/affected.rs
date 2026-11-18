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

/// File nằm ngoài thư mục crate nhưng có test của một crate đọc (bản sinh từ code phải khớp
/// bản đã commit): đổi file thì chạy test của crate đó.
const CHECKED_FILES: &[(&str, &str)] =
    &[("docs/openapi.json", "autoxide"), (".env.example", "autoxide")];

#[derive(Debug, PartialEq, Eq)]
pub enum Affected {
    /// Đổi thứ dùng chung (manifest gốc, `Cargo.lock`, toolchain…): chạy mọi test.
    All,
    /// Tên các crate cần chạy test; rỗng khi chỉ sửa tài liệu.
    Crates(BTreeSet<String>),
}

/// Crate bị ảnh hưởng bởi các file `changed` (đường dẫn tương đối so với gốc repo, dùng `/`).
pub fn affected(workspace: &Workspace, changed: &[String]) -> Affected {
    let mut crates = BTreeSet::new();
    for file in changed {
        if GLOBAL_FILES.contains(&file.as_str())
            || GLOBAL_DIRS.iter().any(|dir| file.starts_with(dir))
        {
            return Affected::All;
        }
        if let Some((_, package)) = CHECKED_FILES.iter().find(|(path, _)| path == file) {
            crates.insert((*package).to_owned());
            continue;
        }
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
pub fn run(workspace: &Workspace, base: Option<&str>, keep_env: bool) -> Result<(), String> {
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
            if !keep_env {
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

    let jobs: Vec<Job> = build_tests(&workspace.root)?
        .into_iter()
        .filter_map(|test| {
            let (name, dir) = dirs.iter().find(|(_, dir)| *dir == test.dir)?;
            Some(Job {
                name: name.clone(),
                dir: dir.clone(),
                executable: test.executable,
                threads: 0,
            })
        })
        .collect();
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
        if !keep_env {
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
/// doc-test (gate chạy một lần, ở lượt SQLite).
pub fn run_all(workspace: &Workspace, doc: bool) -> Result<(), String> {
    let workers = workers();
    let mode = Mode::from_env();
    let mut jobs: Vec<Job> = build_tests(&workspace.root)?
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
            })
        })
        .collect();
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
    if doc {
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
    let total: usize = counts.iter().sum::<usize>().max(1);
    for (job, count) in jobs.iter_mut().zip(counts) {
        job.threads =
            (POSTGRES_CONCURRENT_TESTS * count).div_ceil(total).clamp(1, POSTGRES_CONCURRENT_TESTS);
    }
    // Chương trình lớn chạy trước: nó quyết định tổng thời gian.
    jobs.sort_by_key(|job| std::cmp::Reverse(job.threads));
    Ok(())
}

/// Số test của một chương trình test (`--list`, không chạy gì).
fn count_tests(job: &Job) -> Result<usize, String> {
    let output = without_cargo_run_env(&mut Command::new(&job.executable))
        .args(["--list", "--format", "terse"])
        .current_dir(&job.dir)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| {
            format!("cannot list the tests of {}: {error}", job.executable.display())
        })?;
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| line.ends_with(": test"))
        .count())
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

/// Build mọi file test của workspace (như gate đầy đủ, để dùng chung cache) và trả danh sách.
fn build_tests(root: &Path) -> Result<Vec<TestBinary>, String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let output = without_cargo_run_env(&mut Command::new(&cargo))
        .args([
            "test",
            "--workspace",
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

    #[test]
    fn thu_muc_trung_tien_to_khong_bi_nham() {
        // `modules/notes-extra` không thuộc `modules/notes`.
        assert_eq!(changed(&["modules/notes-extra/README.md"]), crates(&[]));
    }
}
