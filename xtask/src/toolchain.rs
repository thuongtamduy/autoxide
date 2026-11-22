//! Luật `toolchain`: mọi chỗ ghim phiên bản Rust khớp `rust-toolchain.toml` gốc
//! (D-20261122-toolchain-sync).
//!
//! - `examples/external-module/rust-toolchain.toml` cùng `channel` với gốc: ví dụ (và khung
//!   `cargo xtask new-module --external` sinh từ nó) build bằng đúng toolchain của core; lệch thì máy
//!   nào chạy `./gate.sh --only example` cũng phải tải thêm một toolchain.
//! - `rust-version` của `examples/external-module/Cargo.toml` bằng của `Cargo.toml` gốc (MSRV mà khung
//!   sinh ra hứa với người dùng).
//! - `ARG RUST_VERSION` của `Dockerfile` bằng `channel` gốc (ảnh Docker build bằng toolchain đã gate).
//!
//! Nâng toolchain là sửa các chỗ này cùng một lượt; quên chỗ nào thì gate đỏ, chỉ đúng file.

use std::fs;
use std::path::Path;

use crate::boundaries::Violation;

const ROOT_TOOLCHAIN: &str = "rust-toolchain.toml";
const EXAMPLE_TOOLCHAIN: &str = "examples/external-module/rust-toolchain.toml";
const EXAMPLE_MANIFEST: &str = "examples/external-module/Cargo.toml";
const DOCKERFILE: &str = "Dockerfile";

pub fn check(root: &Path) -> Result<Vec<Violation>, String> {
    let read = |relative: &str| {
        fs::read_to_string(root.join(relative))
            .map_err(|error| format!("cannot read {relative}: {error}"))
    };
    let channel = quoted_value(&read(ROOT_TOOLCHAIN)?, "channel")
        .ok_or_else(|| format!("{ROOT_TOOLCHAIN}: no `channel = \"…\"`"))?;
    let rust_version = quoted_value(&read("Cargo.toml")?, "rust-version")
        .ok_or_else(|| "Cargo.toml: no `rust-version = \"…\"`".to_owned())?;
    let mut violations = Vec::new();
    let mut expect = |file: &str, found: Option<String>, wanted: &str, what: &str| {
        if found.as_deref() != Some(wanted) {
            violations.push(Violation {
                rule: "toolchain",
                location: file.to_owned(),
                message: format!(
                    "{what} is {}, expected {wanted} (as in the root of the repository); upgrade \
                     every pinned Rust version together",
                    found.as_deref().unwrap_or("missing")
                ),
            });
        }
    };
    if root.join(EXAMPLE_TOOLCHAIN).exists() {
        let found = quoted_value(&read(EXAMPLE_TOOLCHAIN)?, "channel");
        expect(EXAMPLE_TOOLCHAIN, found, &channel, "`channel`");
    }
    if root.join(EXAMPLE_MANIFEST).exists() {
        let found = quoted_value(&read(EXAMPLE_MANIFEST)?, "rust-version");
        expect(EXAMPLE_MANIFEST, found, &rust_version, "`rust-version`");
    }
    if root.join(DOCKERFILE).exists() {
        let found = docker_arg(&read(DOCKERFILE)?, "RUST_VERSION");
        expect(DOCKERFILE, found, &channel, "`ARG RUST_VERSION`");
    }
    Ok(violations)
}

/// Giá trị của dòng `key = "…"` đầu tiên (bỏ khoảng trắng đầu dòng).
fn quoted_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let rest = line.trim_start().strip_prefix(key)?.trim_start().strip_prefix('=')?;
        let rest = rest.trim().strip_prefix('"')?;
        Some(rest[..rest.find('"')?].to_owned())
    })
}

/// Giá trị mặc định của `ARG <name>=…` trong Dockerfile.
fn docker_arg(text: &str, name: &str) -> Option<String> {
    let prefix = format!("ARG {name}=");
    text.lines().find_map(|line| line.trim().strip_prefix(&prefix).map(|value| value.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, relative: &str, text: &str) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn fixture(example_channel: &str, example_msrv: &str, docker: &str) -> Vec<Violation> {
        let root = std::env::temp_dir().join(format!(
            "autoxide-toolchain-{}-{example_channel}-{example_msrv}-{docker}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        write(&root, ROOT_TOOLCHAIN, "[toolchain]\nchannel = \"1.100.0\"\n");
        write(&root, "Cargo.toml", "[workspace.package]\nrust-version = \"1.99\"\n");
        write(&root, EXAMPLE_TOOLCHAIN, &format!("[toolchain]\nchannel = \"{example_channel}\"\n"));
        write(&root, EXAMPLE_MANIFEST, &format!("rust-version = \"{example_msrv}\"\n"));
        write(&root, DOCKERFILE, &format!("# x\nARG RUST_VERSION={docker}\nFROM rust\n"));
        let violations = check(&root).unwrap();
        fs::remove_dir_all(&root).unwrap();
        violations
    }

    #[test]
    fn khop_thi_khong_vi_pham() {
        assert_eq!(fixture("1.100.0", "1.99", "1.100.0"), []);
    }

    #[test]
    fn lech_thi_chi_dung_file() {
        let locations: Vec<String> = fixture("1.99.1", "1.98", "1.99.0")
            .into_iter()
            .map(|violation| violation.location)
            .collect();
        assert_eq!(locations, [EXAMPLE_TOOLCHAIN, EXAMPLE_MANIFEST, DOCKERFILE]);
    }

    #[test]
    fn doc_gia_tri() {
        assert_eq!(quoted_value("  channel = \"1.2.3\" # x", "channel").as_deref(), Some("1.2.3"));
        assert_eq!(quoted_value("channels = \"1\"", "channel"), None);
        assert_eq!(
            docker_arg("ARG RUST_VERSION=1.100.0", "RUST_VERSION").as_deref(),
            Some("1.100.0")
        );
    }
}
