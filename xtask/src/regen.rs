//! `cargo xtask regen [--check]`: sinh lại MỘT lần mọi file tự động của repo, theo đúng thứ tự phụ thuộc, thay cho
//! chuỗi lệnh chạy tay (D-20261125-regen-and-gate-order):
//!
//! - `docs/openapi.json` ← `autoxide openapi`
//! - `docs/mcp-tools.json` ← `autoxide mcp-manifest`
//! - `.env.example` ← `autoxide config --env-example`
//! - `sdk/typescript/src/schema.ts` ← `cargo xtask sdk` (đọc `docs/openapi.json` vừa sinh, nên đứng sau nó)
//! - `docs/decisions/INDEX.md` ← `cargo xtask decisions` (không nằm trong git, D-20261125-decision-index-untracked)
//!
//! `--check`: chỉ so ba file đầu (cần dựng binary `autoxide`, cùng bản dựng mà bước test sau đó dùng) và in ĐÚNG
//! lệnh phải chạy khi lệch, không ghi gì. SDK đã có `cargo xtask sdk --check` riêng, rẻ hơn.
//!
//! Binary chạy CÔ LẬP: thư mục tạm rỗng làm thư mục hiện tại và môi trường trống. Lý do: `autoxide` đọc `.env` ở thư
//! mục hiện tại và `DATABASE_URL`, và các lệnh này vẫn mở (tạo) database đó — đo: `.env` có `DATABASE_URL` làm lệnh tạo
//! file database ở đường dẫn ấy. Chạy ở gốc repo thì `.env` và `DATABASE_URL` của người phát triển (có thể trỏ vào một
//! database thật) lọt vào; nay chúng không với tới được.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Deserialize;

/// (file, tham số của `autoxide`).
const OUTPUTS: &[(&str, &[&str])] = &[
    ("docs/openapi.json", &["openapi"]),
    ("docs/mcp-tools.json", &["mcp-manifest"]),
    (".env.example", &["config", "--env-example"]),
];

/// Lệnh người dùng chạy khi `--check` báo lệch.
pub const FIX_COMMAND: &str = "cargo xtask regen";

/// Dòng báo một file lệch (`--check`), có lệnh sửa.
pub fn stale_message(file: &str) -> String {
    format!("[generated-file] {file}: out of date — run `{FIX_COMMAND}` and commit the result")
}

/// Sinh (hay so) ba file từ binary `autoxide`. Trả danh sách file lệch (rỗng: khớp hết); không `check` thì đã ghi.
pub fn outputs(root: &Path, check: bool) -> Result<Vec<&'static str>, String> {
    let binary = build_binary(root)?;
    let sandbox = Sandbox::new()?;
    let mut generated = Vec::new();
    for (file, args) in OUTPUTS {
        generated.push((*file, run(&binary, sandbox.path(), args)?));
    }
    apply(root, &generated, check)
}

/// Phần thuần: so từng file với nội dung sinh; không `check` thì ghi các file lệch. Trả danh sách file lệch.
fn apply(
    root: &Path,
    generated: &[(&'static str, String)],
    check: bool,
) -> Result<Vec<&'static str>, String> {
    let mut stale = Vec::new();
    for (file, content) in generated {
        let current = fs::read_to_string(root.join(file)).unwrap_or_default();
        if current != *content {
            stale.push(*file);
            if !check {
                fs::write(root.join(file), content).map_err(|error| format!("{file}: {error}"))?;
            }
        }
    }
    Ok(stale)
}

#[derive(Deserialize)]
struct Message {
    reason: String,
    executable: Option<PathBuf>,
    target: Option<Target>,
}

#[derive(Deserialize)]
struct Target {
    name: String,
    kind: Vec<String>,
}

/// Dựng binary `autoxide` (cargo chạy ở gốc repo để theo `rust-toolchain.toml`) và trả đường dẫn của nó.
fn build_binary(root: &Path) -> Result<PathBuf, String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let output = Command::new(&cargo)
        .args(["build", "--locked", "-p", "autoxide", "--bin", "autoxide"])
        .arg("--message-format=json-render-diagnostics")
        .current_dir(root)
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|error| format!("cannot run `{cargo} build`: {error}"))?;
    if !output.status.success() {
        return Err("building `autoxide` failed (see the errors above)".to_owned());
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<Message>(line).ok())
        .filter(|message| message.reason == "compiler-artifact")
        .filter(|message| {
            message.target.as_ref().is_some_and(|target| {
                target.name == "autoxide" && target.kind.contains(&"bin".to_owned())
            })
        })
        .find_map(|message| message.executable)
        .ok_or_else(|| "cargo did not report the `autoxide` binary".to_owned())
}

/// Thư mục tạm rỗng, xoá khi hết dùng.
struct Sandbox(PathBuf);

impl Sandbox {
    fn new() -> Result<Self, String> {
        let path = std::env::temp_dir().join(format!("autoxide-regen-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        Ok(Sandbox(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Lệnh chạy binary cô lập: thư mục hiện tại là `dir` (rỗng, không có `.env`), môi trường trống.
fn isolated(binary: &Path, dir: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(binary);
    command.args(args).current_dir(dir).env_clear().stdin(Stdio::null());
    command
}

fn run(binary: &Path, dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = isolated(binary, dir, args)
        .output()
        .map_err(|error| format!("cannot run {}: {error}", binary.display()))?;
    if !output.status.success() {
        // Cảnh báo lúc khởi động (thiếu khoá…) chỉ nhiễu khi thành công; lỗi thì in hết.
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
        return Err(format!("`autoxide {}` failed (see above)", args.join(" ")));
    }
    String::from_utf8(output.stdout)
        .map_err(|error| format!("autoxide output is not UTF-8: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("xtask-regen-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(path.join("docs")).unwrap();
        path
    }

    fn sample() -> Vec<(&'static str, String)> {
        vec![("docs/openapi.json", "new".to_owned()), ("docs/mcp-tools.json", "same".to_owned())]
    }

    #[test]
    fn check_bao_lech_va_khong_ghi_gi() {
        let root = temp("check");
        fs::write(root.join("docs/openapi.json"), "old").unwrap();
        fs::write(root.join("docs/mcp-tools.json"), "same").unwrap();

        let stale = apply(&root, &sample(), true).unwrap();

        assert_eq!(stale, ["docs/openapi.json"]);
        assert_eq!(
            fs::read_to_string(root.join("docs/openapi.json")).unwrap(),
            "old",
            "--check không ghi"
        );
        let message = stale_message(stale[0]);
        assert!(
            message.contains("docs/openapi.json") && message.contains("`cargo xtask regen`"),
            "{message}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn khong_check_thi_ghi_file_lech_va_bo_qua_file_khop() {
        let root = temp("write");
        fs::write(root.join("docs/mcp-tools.json"), "same").unwrap();

        let stale = apply(&root, &sample(), false).unwrap();

        assert_eq!(stale, ["docs/openapi.json"], "file thiếu coi như lệch");
        assert_eq!(fs::read_to_string(root.join("docs/openapi.json")).unwrap(), "new");
        assert!(apply(&root, &sample(), true).unwrap().is_empty(), "ghi xong thì khớp");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn binary_chay_trong_thu_muc_tam_rong_khong_co_env() {
        let sandbox = Sandbox::new().unwrap();
        assert!(fs::read_dir(sandbox.path()).unwrap().next().is_none(), "thư mục tạm rỗng");
        assert!(!sandbox.path().join(".env").exists());
        let command = isolated(Path::new("autoxide"), sandbox.path(), &["openapi"]);
        assert_eq!(command.get_current_dir(), Some(sandbox.path()));
        assert_ne!(
            Some(sandbox.path()),
            std::env::current_dir().ok().as_deref(),
            "không chạy ở thư mục hiện tại"
        );
    }
}
