//! Đọc cấu trúc workspace từ `cargo metadata` thành một mô hình nhỏ mà các luật
//! trong `boundaries.rs` kiểm trên đó.
//!
//! Tách "đọc" khỏi "kiểm" để phần kiểm là hàm thuần: test dựng mô hình bằng tay,
//! không cần tạo workspace thật trên đĩa.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

/// Vai của một crate, suy ra từ vị trí thư mục của nó trong repo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// `crates/kernel` — nền mà mọi module dựa vào.
    Kernel,
    /// `crates/testkit` — đồ dùng cho test.
    Testkit,
    /// `crates/app` — binary, nơi đăng ký module.
    App,
    /// `crates/macros` — macro thủ tục của kernel; module dùng qua `autoxide_kernel`, không khai trực tiếp.
    Macros,
    /// `modules/<tên>` — một module.
    Module,
    /// `xtask` — công cụ cho người phát triển.
    Tool,
    /// Vị trí không thuộc bố cục đã quy ước.
    Unknown,
}

impl Role {
    /// `dir` là đường dẫn thư mục crate, tương đối so với gốc workspace, dùng `/`.
    pub fn from_dir(dir: &str) -> Role {
        match dir {
            "crates/kernel" => Role::Kernel,
            "crates/testkit" => Role::Testkit,
            "crates/app" => Role::App,
            "crates/macros" => Role::Macros,
            "xtask" => Role::Tool,
            _ => match dir.strip_prefix("modules/") {
                // Module nằm ngay dưới modules/, không lồng thêm cấp.
                Some(name) if !name.is_empty() && !name.contains('/') => Role::Module,
                _ => Role::Unknown,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepKind {
    Normal,
    Dev,
    Build,
}

#[derive(Debug, Clone)]
pub struct Dep {
    pub name: String,
    pub kind: DepKind,
}

#[derive(Debug, Clone)]
pub struct Package {
    pub name: String,
    pub license: Option<String>,
    /// Thư mục crate, tương đối so với gốc workspace, dùng `/`.
    pub dir: String,
    pub role: Role,
    /// Mọi dependency khai trong manifest (cả crate ngoài lẫn crate nội bộ).
    pub deps: Vec<Dep>,
}

impl Package {
    /// Tên crate khi viết trong code Rust: dấu `-` thành `_`.
    pub fn ident(&self) -> String {
        self.name.replace('-', "_")
    }
}

#[derive(Debug, Clone)]
pub struct Workspace {
    pub root: PathBuf,
    pub packages: Vec<Package>,
}

impl Workspace {
    pub fn package(&self, name: &str) -> Option<&Package> {
        self.packages.iter().find(|package| package.name == name)
    }
}

// Phần dưới ánh xạ đúng những trường cần dùng trong JSON của `cargo metadata`.
// serde bỏ qua các trường không khai.
#[derive(Deserialize)]
struct RawMetadata {
    workspace_root: PathBuf,
    packages: Vec<RawPackage>,
}

#[derive(Deserialize)]
struct RawPackage {
    name: String,
    license: Option<String>,
    manifest_path: PathBuf,
    dependencies: Vec<RawDep>,
}

#[derive(Deserialize)]
struct RawDep {
    name: String,
    /// `null` = dependency thường, `"dev"`, hoặc `"build"`.
    kind: Option<String>,
}

/// Chạy `cargo metadata` cho workspace hiện tại.
pub fn load() -> Result<Workspace, String> {
    // Khi chạy qua `cargo xtask`, cargo đặt biến CARGO trỏ tới chính nó — dùng lại
    // để chắc chắn cùng toolchain đang ghim.
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let output = Command::new(&cargo)
        // --no-deps: chỉ liệt kê crate của workspace, không tải cây phụ thuộc.
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .map_err(|error| format!("cannot run `{cargo} metadata`: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "`{cargo} metadata` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let json = String::from_utf8(output.stdout)
        .map_err(|error| format!("`cargo metadata` output is not UTF-8: {error}"))?;
    parse(&json)
}

/// Chuyển JSON của `cargo metadata --no-deps` thành [`Workspace`].
pub fn parse(json: &str) -> Result<Workspace, String> {
    let raw: RawMetadata = serde_json::from_str(json)
        .map_err(|error| format!("cannot parse `cargo metadata` output: {error}"))?;
    let mut packages = Vec::with_capacity(raw.packages.len());
    for package in raw.packages {
        let dir = relative_dir(&raw.workspace_root, &package.manifest_path)?;
        let deps = package
            .dependencies
            .into_iter()
            .map(|dep| Dep {
                name: dep.name,
                kind: match dep.kind.as_deref() {
                    Some("dev") => DepKind::Dev,
                    Some("build") => DepKind::Build,
                    _ => DepKind::Normal,
                },
            })
            .collect();
        packages.push(Package {
            name: package.name,
            license: package.license,
            role: Role::from_dir(&dir),
            dir,
            deps,
        });
    }
    // Sắp xếp để thứ tự báo lỗi ổn định giữa các lần chạy.
    packages.sort_by(|a, b| a.dir.cmp(&b.dir));
    Ok(Workspace { root: raw.workspace_root, packages })
}

/// Thư mục chứa `manifest_path`, tương đối so với `root`, luôn dùng `/`.
fn relative_dir(root: &Path, manifest_path: &Path) -> Result<String, String> {
    let dir = manifest_path
        .parent()
        .ok_or_else(|| format!("manifest path has no parent: {}", manifest_path.display()))?;
    let relative = dir.strip_prefix(root).map_err(|_| {
        format!("crate {} is outside the workspace root {}", dir.display(), root.display())
    })?;
    let parts: Vec<String> =
        relative.components().map(|part| part.as_os_str().to_string_lossy().into_owned()).collect();
    Ok(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vai_suy_ra_tu_thu_muc() {
        assert_eq!(Role::from_dir("crates/kernel"), Role::Kernel);
        assert_eq!(Role::from_dir("crates/testkit"), Role::Testkit);
        assert_eq!(Role::from_dir("crates/app"), Role::App);
        assert_eq!(Role::from_dir("modules/auth"), Role::Module);
        assert_eq!(Role::from_dir("xtask"), Role::Tool);
    }

    #[test]
    fn thu_muc_la_khong_co_vai() {
        assert_eq!(Role::from_dir("crates/other"), Role::Unknown);
        assert_eq!(Role::from_dir("modules/auth/nested"), Role::Unknown);
        assert_eq!(Role::from_dir("modules/"), Role::Unknown);
        assert_eq!(Role::from_dir(""), Role::Unknown);
    }

    #[test]
    fn doc_duoc_json_cua_cargo_metadata() {
        let json = r#"{
            "workspace_root": "/repo",
            "packages": [
                {
                    "name": "autoxide-auth",
                    "license": "MIT OR Apache-2.0",
                    "manifest_path": "/repo/modules/auth/Cargo.toml",
                    "dependencies": [
                        {"name": "autoxide-kernel", "kind": null},
                        {"name": "autoxide-testkit", "kind": "dev"},
                        {"name": "cc", "kind": "build"}
                    ]
                },
                {
                    "name": "autoxide-kernel",
                    "license": null,
                    "manifest_path": "/repo/crates/kernel/Cargo.toml",
                    "dependencies": []
                }
            ]
        }"#;
        let workspace = parse(json).unwrap();

        assert_eq!(workspace.root, PathBuf::from("/repo"));
        // Đã sắp theo thư mục: crates/kernel đứng trước modules/auth.
        let kernel = &workspace.packages[0];
        assert_eq!((kernel.dir.as_str(), kernel.role), ("crates/kernel", Role::Kernel));
        assert_eq!(kernel.license, None);

        let auth = workspace.package("autoxide-auth").unwrap();
        assert_eq!(auth.role, Role::Module);
        assert_eq!(auth.ident(), "autoxide_auth");
        let kinds: Vec<DepKind> = auth.deps.iter().map(|dep| dep.kind).collect();
        assert_eq!(kinds, [DepKind::Normal, DepKind::Dev, DepKind::Build]);
    }

    #[test]
    fn crate_ngoai_workspace_la_loi() {
        let json = r#"{
            "workspace_root": "/repo",
            "packages": [{
                "name": "x", "license": null,
                "manifest_path": "/elsewhere/x/Cargo.toml", "dependencies": []
            }]
        }"#;
        assert!(parse(json).unwrap_err().contains("outside the workspace root"));
    }
}
