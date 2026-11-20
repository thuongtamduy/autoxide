//! `cargo xtask upgrade-test [--from <tag>] [--image <image>] [--database sqlite|postgres|both]`:
//! chạy test nâng cấp (`crates/app/tests/upgrade.rs`, đặc tả 31 B1, D-246) — bản phát hành trước
//! ghi dữ liệu, bản hiện tại đọc lại trên cùng database.
//!
//! 1. Tag cũ: `--from`, không có thì tag phát hành gần nhất trước HEAD (như `api-diff`; HEAD chính
//!    là một tag thì lấy tag trước nó).
//! 2. Image cũ: `--image`; không có thì `docker pull ghcr.io/<chủ>/<repo>:X.Y.Z` (image mà
//!    quy trình phát hành đẩy lên); không tải được thì dùng `autoxide:vX.Y.Z` đã build trên máy, hay
//!    build nó từ tag (`git worktree add` → `docker build`).
//! 3. `cargo test -p autoxide --test integration upgrade::` với `TEST_UPGRADE_IMAGE`: trên SQLite, rồi trên
//!    Postgres (`GATE_POSTGRES_URL`, không có thì dựng container như `gate.sh`).
//!
//! Cần Docker, build bản release của tag cũ, lâu: chạy trong job `upgrade` của CI và lúc phát
//! hành, không trong `./gate.sh`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// Image Postgres khi phải tự dựng (cùng mặc định với `gate.sh`).
const POSTGRES_IMAGE: &str = "postgres:16-alpine";

/// Database cần chạy test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Databases {
    Sqlite,
    Postgres,
    Both,
}

impl Databases {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "sqlite" => Some(Databases::Sqlite),
            "postgres" => Some(Databases::Postgres),
            "both" => Some(Databases::Both),
            _ => None,
        }
    }
}

/// Tham số của lệnh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub from: Option<String>,
    pub image: Option<String>,
    pub databases: Databases,
}

/// Đọc tham số sau `upgrade-test`.
pub fn parse(args: &[&str]) -> Result<Options, String> {
    let mut options = Options { from: None, image: None, databases: Databases::Both };
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value =
            || args.next().map(|value| (*value).to_owned()).ok_or(format!("{arg} needs a value"));
        match *arg {
            "--from" => options.from = Some(value()?),
            "--image" => options.image = Some(value()?),
            "--database" => {
                let text = value()?;
                options.databases = Databases::parse(&text).ok_or(format!(
                    "--database takes `sqlite`, `postgres` or `both`, not `{text}`"
                ))?;
            }
            other => return Err(format!("unknown argument `{other}`")),
        }
    }
    Ok(options)
}

/// `owner/repo` từ địa chỉ remote của git (`https://github.com/o/r.git`, `git@github.com:o/r.git`,
/// hay qua proxy `http://…/git/o/r`): hai đoạn cuối của đường dẫn, viết thường (tên image trên
/// ghcr.io phải viết thường).
pub fn repository(remote: &str) -> Option<String> {
    let path = remote.trim().trim_end_matches('/').trim_end_matches(".git").replace(':', "/");
    let mut parts = path.rsplit('/').filter(|part| !part.is_empty());
    let repo = parts.next()?;
    let owner = parts.next()?;
    Some(format!("{owner}/{repo}").to_lowercase())
}

/// Tên image có thể có trên ghcr.io cho tag `vX.Y.Z`: `X.Y.Z` (như `docs/RELEASING.md` ghi) và
/// chính tên tag (thẻ mặc định của `docker/metadata-action` cho lần đẩy tag).
pub fn registry_images(repository: &str, tag: &str) -> Vec<String> {
    let bare = tag.trim_start_matches('v');
    let mut images = vec![format!("ghcr.io/{repository}:{bare}")];
    if bare != tag {
        images.push(format!("ghcr.io/{repository}:{tag}"));
    }
    images
}

/// Tên image build trên máy từ tag.
pub fn local_image(tag: &str) -> String {
    format!("autoxide:{tag}")
}

fn run_command(command: &mut Command, what: &str) -> Result<(), String> {
    let status = command.status().map_err(|error| format!("cannot run {what}: {error}"))?;
    if status.success() { Ok(()) } else { Err(format!("{what} failed ({status})")) }
}

fn output(command: &mut Command) -> Option<String> {
    let output = command.output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
    output(Command::new("git").current_dir(root).args(args))
}

/// Tag phát hành gần nhất **trước** HEAD.
fn previous_tag(root: &Path) -> Result<String, String> {
    let last = crate::api_diff::last_release_tag(root)?
        .ok_or("no release tag before HEAD; pass `--from vX.Y.Z`")?;
    let at_head = git(root, &["rev-list", "-n", "1", &last]) == git(root, &["rev-parse", "HEAD"]);
    if !at_head {
        return Ok(last);
    }
    git(
        root,
        &[
            "describe",
            "--tags",
            "--abbrev=0",
            "--match",
            "v[0-9]*.[0-9]*.[0-9]*",
            &format!("{last}^"),
        ],
    )
    .ok_or(format!("HEAD is {last} and there is no release tag before it; pass `--from vX.Y.Z`"))
}

/// Biến môi trường có giá trị (công cụ của người phát triển, không phải cấu hình chương trình).
fn variable(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

/// Image của bản cũ: tải về, hay có sẵn trên máy, hay build từ tag.
fn old_image(root: &Path, tag: &str) -> Result<String, String> {
    let repository = variable("GITHUB_REPOSITORY")
        .map(|value| value.to_lowercase())
        .or_else(|| git(root, &["remote", "get-url", "origin"]).and_then(|url| repository(&url)));
    for image in repository.iter().flat_map(|repository| registry_images(repository, tag)) {
        println!("upgrade-test: pulling {image}");
        if run_command(Command::new("docker").args(["pull", "--quiet", &image]), "docker pull")
            .is_ok()
        {
            return Ok(image);
        }
    }
    println!(
        "upgrade-test: no published image of {tag} could be pulled; using a local build. To publish \
         it, run the GitHub workflow \"Release image (existing tag)\" with tag {tag} \
         (D-20261120-upgrade-image)"
    );
    let image = local_image(tag);
    if output(Command::new("docker").args(["image", "inspect", "--format", "{{.Id}}", &image]))
        .is_some()
    {
        println!("upgrade-test: using {image} built earlier on this machine");
        return Ok(image);
    }
    let directory =
        std::env::temp_dir().join(format!("autoxide-upgrade-{tag}-{}", std::process::id()));
    let worktree = Worktree::add(root, &directory, tag)?;
    let mut build = Command::new("docker");
    build.args(["build", "--network", "host", "-t", &image]);
    // Máy sau proxy (biến chuẩn): truyền cho bước build tải thư viện.
    for name in ["HTTPS_PROXY", "HTTP_PROXY", "NO_PROXY"] {
        if let Some(value) = variable(name) {
            build.args(["--build-arg", &format!("{name}={value}")]);
        }
    }
    build.arg(".").current_dir(&worktree.0);
    println!("upgrade-test: building {image} from {tag} (a release build; this takes a while)");
    run_command(&mut build, "docker build")?;
    Ok(image)
}

/// Worktree tạm của tag cũ; gỡ khi xong.
struct Worktree(PathBuf, PathBuf);

impl Worktree {
    fn add(root: &Path, directory: &Path, tag: &str) -> Result<Self, String> {
        let path = directory.display().to_string();
        run_command(
            Command::new("git").current_dir(root).args(["worktree", "add", "--detach", &path, tag]),
            "git worktree add",
        )?;
        Ok(Worktree(directory.to_owned(), root.to_owned()))
    }
}

impl Drop for Worktree {
    fn drop(&mut self) {
        let path = self.0.display().to_string();
        let _ = Command::new("git")
            .current_dir(&self.1)
            .args(["worktree", "remove", "--force", &path])
            .status();
    }
}

/// Container Postgres tự dựng; gỡ khi xong.
struct Postgres(String);

impl Drop for Postgres {
    fn drop(&mut self) {
        let _ = Command::new("docker").args(["rm", "--force", "--volumes", &self.0]).output();
    }
}

fn start_postgres() -> Result<(Postgres, String), String> {
    let name = format!("autoxide-upgrade-postgres-{}", std::process::id());
    let password = "postgres";
    run_command(
        Command::new("docker").args([
            "run",
            "--detach",
            "--name",
            &name,
            "--env",
            &format!("POSTGRES_PASSWORD={password}"),
            "--publish",
            "127.0.0.1::5432",
            POSTGRES_IMAGE,
        ]),
        "docker run postgres",
    )?;
    let container = Postgres(name);
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(60) {
        let ready = Command::new("docker")
            .args([
                "exec",
                &container.0,
                "pg_isready",
                "--quiet",
                "-h",
                "127.0.0.1",
                "-U",
                "postgres",
            ])
            .status()
            .is_ok_and(|status| status.success());
        if ready
            && let Some(port) =
                output(Command::new("docker").args(["port", &container.0, "5432/tcp"])).and_then(
                    |line| {
                        line.lines()
                            .next()
                            .and_then(|line| line.rsplit_once(':'))
                            .map(|(_, port)| port.to_owned())
                    },
                )
        {
            let url = format!("postgres://postgres:{password}@127.0.0.1:{port}/postgres");
            return Ok((container, url));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err("Postgres did not become ready within 60 seconds".to_owned())
}

fn cargo_test(root: &Path, image: &str, database_url: Option<&str>) -> Result<(), String> {
    let cargo = variable("CARGO").unwrap_or_else(|| "cargo".to_owned());
    let mut command = Command::new(cargo);
    command
        .current_dir(root)
        // Test tích hợp của `autoxide` là một chương trình (`tests/integration/`, D-20261118-build-size):
        // lọc đúng module `upgrade` của nó.
        .args([
            "test",
            "--locked",
            "-p",
            "autoxide",
            "--test",
            "integration",
            "upgrade::",
            "--",
            "--nocapture",
        ])
        .env("TEST_UPGRADE_IMAGE", image)
        .env_remove("TEST_DATABASE_URL");
    if let Some(url) = database_url {
        command.env("TEST_DATABASE_URL", url);
    }
    run_command(&mut command, "cargo test --test integration upgrade::")
}

/// Chạy lệnh. `Ok(())` khi test xanh trên mọi database đã chọn.
pub fn run(root: &Path, options: &Options) -> Result<(), String> {
    let image = match &options.image {
        Some(image) => image.clone(),
        None => {
            let tag = match &options.from {
                Some(tag) => tag.clone(),
                None => previous_tag(root)?,
            };
            println!("upgrade-test: upgrading from {tag}");
            old_image(root, &tag)?
        }
    };
    if matches!(options.databases, Databases::Sqlite | Databases::Both) {
        println!("upgrade-test: SQLite");
        cargo_test(root, &image, None)?;
    }
    if matches!(options.databases, Databases::Postgres | Databases::Both) {
        match variable("GATE_POSTGRES_URL") {
            Some(url) => {
                println!("upgrade-test: Postgres (GATE_POSTGRES_URL)");
                cargo_test(root, &image, Some(&url))?;
            }
            None => {
                println!("upgrade-test: Postgres ({POSTGRES_IMAGE} in Docker)");
                let (_container, url) = start_postgres()?;
                cargo_test(root, &image, Some(&url))?;
            }
        }
    }
    println!("upgrade-test: green");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_tham_so() {
        assert_eq!(
            parse(&[]).unwrap(),
            Options { from: None, image: None, databases: Databases::Both }
        );
        assert_eq!(
            parse(&["--from", "v0.13.0", "--database", "postgres"]).unwrap(),
            Options {
                from: Some("v0.13.0".to_owned()),
                image: None,
                databases: Databases::Postgres
            }
        );
        assert_eq!(parse(&["--image", "x:1"]).unwrap().image.as_deref(), Some("x:1"));
        assert!(parse(&["--database", "mysql"]).unwrap_err().contains("sqlite"));
        assert!(parse(&["--from"]).unwrap_err().contains("needs a value"));
        assert!(parse(&["--fast"]).unwrap_err().contains("unknown"));
    }

    #[test]
    fn lay_chu_repo_tu_dia_chi_remote() {
        for (remote, expected) in [
            ("https://github.com/Owner/Autoxide.git", "owner/autoxide"),
            ("git@github.com:owner/autoxide.git", "owner/autoxide"),
            ("http://127.0.0.1:8080/git/owner/autoxide", "owner/autoxide"),
            ("https://github.com/owner/autoxide/", "owner/autoxide"),
        ] {
            assert_eq!(repository(remote).as_deref(), Some(expected), "{remote}");
        }
        assert_eq!(repository("autoxide"), None);
    }

    #[test]
    fn ten_image_theo_tag() {
        assert_eq!(
            registry_images("owner/autoxide", "v0.13.0"),
            ["ghcr.io/owner/autoxide:0.13.0", "ghcr.io/owner/autoxide:v0.13.0"]
        );
        assert_eq!(local_image("v0.13.0"), "autoxide:v0.13.0");
    }

    /// Hai workflow đăng image (phát hành, và đăng lại cho tag đã có) đặt đúng tên và thẻ mà
    /// `registry_images` kéo: `ghcr.io/<owner>/<repo>` viết thường, thẻ `X.Y.Z`
    /// (D-20261120-upgrade-image). Trước đây job `upgrade` báo "manifest unknown" vì chưa từng có image.
    #[test]
    fn workflow_dang_image_dung_ten_upgrade_test_keo() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let read = |file: &str| {
            std::fs::read_to_string(root.join(".github/workflows").join(file)).unwrap()
        };
        let image_name = r#"IMAGE=ghcr.io/${GITHUB_REPOSITORY,,}"#;
        let release = read("release.yml");
        assert!(release.contains(image_name));
        assert!(release.contains("type=semver,pattern={{version}}"));
        let again = read("release-image.yml");
        assert!(again.contains(image_name));
        assert!(again.contains(r#"-t "$IMAGE:$VERSION""#));
        assert!(again.contains(r#"echo "version=${TAG#v}""#));
    }
}
