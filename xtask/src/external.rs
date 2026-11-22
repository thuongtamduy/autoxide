//! `cargo xtask check-module <thư mục>`: chạy các luật của `cargo xtask check` áp cho MỘT module trên
//! một crate nằm NGOÀI workspace này — module ở repo riêng của người khác (bước 70 P2,
//! D-20261121-external-modules; điều kiện "luật chạy được trong repo của người viết module" của D-164).
//!
//! Chỉ chạy luật đọc mã nguồn của chính module: bề mặt crate, route, handler không chạm database,
//! lint không bị nới, cấu hình qua `ConfigReader`, câu SQL viết tay, bản dịch, `tenant_id` của
//! entity, lọc theo khách, tool MCP theo phạm vi. Luật về bố cục workspace (license chung, vị trí
//! crate, một chương trình test) là quy ước của repo này, không áp cho người ngoài. Luật nằm trong
//! cấu trúc database (sở hữu bảng, foreign key, index) do test của testkit kiểm, không ở đây.
//!
//! Các luật đều tính đường dẫn từ `<gốc workspace>/modules/<tên>`. Thay vì sửa từng luật, lệnh này
//! dựng một gốc tạm có đúng một symlink `modules/<tên>` trỏ tới crate cần kiểm, rồi đổi đường dẫn
//! trong thông báo về đường dẫn người dùng đã gõ.

use std::path::{Path, PathBuf};

use crate::boundaries::{self, Violation};
use crate::workspace::{Package, Role, Workspace};
use crate::{i18n, mcp_principal, tenancy, tenant_scope};

/// Kiểm crate ở `dir`. Trả về tên module và các vi phạm.
pub fn check(dir: &Path) -> Result<(String, Vec<Violation>), String> {
    let dir =
        dir.canonicalize().map_err(|error| format!("cannot open {}: {error}", dir.display()))?;
    let lib = dir.join("src/lib.rs");
    let text = std::fs::read_to_string(&lib)
        .map_err(|error| format!("cannot read {}: {error}", lib.display()))?;
    let module = module_name(&text).ok_or_else(|| {
        format!(
            "{}: no `fn name(&self) -> &'static str {{ \"…\" }}` of the `Module` implementation \
             found; it must be in src/lib.rs",
            lib.display()
        )
    })?;
    let root = TempRoot::new(&dir, &module)?;
    let package = Package {
        name: module.clone(),
        license: None,
        dir: format!("modules/{module}"),
        role: Role::Module,
        deps: Vec::new(),
    };
    let workspace = Workspace { root: root.0.clone(), packages: vec![package] };
    let mut violations = boundaries::check_external_module(&workspace)?;
    violations.extend(i18n::check_translations(&workspace)?);
    violations.extend(tenancy::check_entities(&workspace)?);
    violations.extend(tenant_scope::check(&workspace)?);
    violations.extend(mcp_principal::check(&workspace)?);
    // `modules/<tên>/src/x.rs:12` → `src/x.rs:12`, tính từ crate người dùng chỉ.
    let prefix = format!("modules/{module}/");
    for violation in &mut violations {
        violation.location = violation.location.replace(&prefix, "");
        violation.message = violation.message.replace(&prefix, "");
    }
    Ok((module, violations))
}

/// Tên module: chuỗi trả về của `fn name(&self)` trong `src/lib.rs` (cùng dòng hay dòng kế, như
/// `cargo fmt` viết).
fn module_name(lib: &str) -> Option<String> {
    let mut lines = lib.lines().skip_while(|line| !line.contains("fn name(&self)"));
    let found: String = lines.by_ref().take(3).collect::<Vec<_>>().join(" ");
    let name = found.split('"').nth(1)?;
    let valid = !name.is_empty()
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    valid.then(|| name.to_owned())
}

/// Gốc workspace tạm: `<tạm>/modules/<tên>` là symlink tới crate. Tự xoá khi xong.
struct TempRoot(PathBuf);

impl TempRoot {
    fn new(target: &Path, module: &str) -> Result<Self, String> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.subsec_nanos());
        let root = std::env::temp_dir()
            .join(format!("autoxide-check-module-{}-{nanos}", std::process::id()));
        let modules = root.join("modules");
        std::fs::create_dir_all(&modules)
            .map_err(|error| format!("cannot create {}: {error}", modules.display()))?;
        let created = TempRoot(root);
        link(target, &modules.join(module))?;
        Ok(created)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        // Xoá symlink trước (không đi theo nó), rồi tới thư mục tạm.
        if let Ok(entries) = std::fs::read_dir(self.0.join("modules")) {
            for entry in entries.flatten() {
                let _ = std::fs::remove_file(entry.path());
            }
        }
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(unix)]
fn link(target: &Path, link: &Path) -> Result<(), String> {
    std::os::unix::fs::symlink(target, link)
        .map_err(|error| format!("cannot link {}: {error}", link.display()))
}

#[cfg(not(unix))]
fn link(_target: &Path, _link: &Path) -> Result<(), String> {
    Err("check-module needs symbolic links; run it on Linux, macOS or WSL".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ten_module_doc_tu_ham_name() {
        let lib = "impl Module for X {\n    fn name(&self) -> &'static str {\n        \
                   \"bookmarks\"\n    }\n    fn description(&self) -> &'static str {\n        \
                   \"Other.\"\n    }\n}\n";
        assert_eq!(module_name(lib), Some("bookmarks".to_owned()));
        assert_eq!(module_name("fn other() {}"), None);
        assert_eq!(module_name("fn name(&self) -> &'static str { \"Not A Name\" }"), None);
    }

    fn crate_with(files: &[(&str, &str)]) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let dir = std::env::temp_dir()
            .join(format!("autoxide-external-test-{}-{nanos}", std::process::id()));
        for (path, text) in files {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        dir
    }

    const LIB: &str = "pub mod api;\nmod service;\npub fn module() -> impl Module {\n    M\n}\n\
                       impl Module for M {\n    fn name(&self) -> &'static str {\n        \
                       \"bookmarks\"\n    }\n}\n";

    #[cfg(unix)]
    #[test]
    fn crate_ngoai_dung_luat_thi_khong_vi_pham() {
        let dir = crate_with(&[
            ("src/lib.rs", LIB),
            ("src/api.rs", "pub trait Bookmarks {}\n"),
            (
                "src/service.rs",
                "fn f() -> Error {\n    Error::not_found(\"bookmarks.missing\", \"Missing.\")\n}\n",
            ),
            ("locales/vi.json", "{\"bookmarks.missing\": \"Không có.\"}"),
        ]);
        let (module, violations) = check(&dir).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(module, "bookmarks");
        assert_eq!(violations, Vec::new());
    }

    #[cfg(unix)]
    #[test]
    fn crate_ngoai_vi_pham_duoc_bao_theo_duong_dan_cua_no() {
        let dir = crate_with(&[
            ("src/lib.rs", &format!("{LIB}pub mod entity;\n")),
            ("src/api.rs", "pub trait Bookmarks {}\n"),
            (
                "src/service.rs",
                "fn f() -> Error {\n    Error::not_found(\"bookmarks.missing\", \"Missing.\")\n}\n",
            ),
            ("src/entity.rs", ""),
        ]);
        let (_, violations) = check(&dir).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let found: Vec<(&str, &str)> = violations
            .iter()
            .map(|violation| (violation.rule, violation.location.as_str()))
            .collect();
        assert!(found.contains(&("module-surface", "src/lib.rs:11")), "{found:?}");
        assert!(found.iter().any(|(rule, _)| rule.starts_with("i18n")), "{found:?}");
    }
}
