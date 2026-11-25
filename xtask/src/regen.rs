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

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

/// (file, tham số của `autoxide`).
const OUTPUTS: &[(&str, &[&str])] = &[
    ("docs/openapi.json", &["openapi"]),
    ("docs/mcp-tools.json", &["mcp-manifest"]),
    (".env.example", &["config", "--env-example"]),
];

/// Lệnh người dùng chạy khi `--check` báo lệch.
pub const FIX_COMMAND: &str = "cargo xtask regen";

/// Sinh (hay so) ba file từ binary `autoxide`. `Ok(danh sách file lệch)`: rỗng là khớp hết; không `check` thì đã ghi
/// và danh sách là các file vừa đổi.
pub fn outputs(root: &Path, check: bool) -> Result<Vec<&'static str>, String> {
    let mut stale = Vec::new();
    for (file, args) in OUTPUTS {
        let generated = generate(root, args)?;
        let current = fs::read_to_string(root.join(file)).unwrap_or_default();
        if current != generated {
            stale.push(*file);
            if !check {
                fs::write(root.join(file), generated)
                    .map_err(|error| format!("{file}: {error}"))?;
            }
        }
    }
    Ok(stale)
}

fn generate(root: &Path, args: &[&str]) -> Result<String, String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let mut command = Command::new(&cargo);
    command.args(["run", "--quiet", "--locked", "-p", "autoxide", "--"]).args(args);
    // Cấu hình của người chạy không được lọt vào file sinh: lấy giá trị mặc định (như các bài test giữ tài liệu).
    for (name, _) in std::env::vars_os() {
        if name.to_str().is_some_and(|name| name.starts_with("AUTOXIDE_")) {
            command.env_remove(name);
        }
    }
    let output = command
        .current_dir(root)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("cannot run `{cargo} run -p autoxide`: {error}"))?;
    if !output.status.success() {
        // Cảnh báo lúc khởi động (thiếu khoá, v.v.) chỉ nhiễu khi thành công; lỗi thì in hết.
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
        return Err(format!("`autoxide {}` failed (see above)", args.join(" ")));
    }
    String::from_utf8(output.stdout)
        .map_err(|error| format!("autoxide output is not UTF-8: {error}"))
}
