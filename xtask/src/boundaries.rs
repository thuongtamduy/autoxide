//! Luật ranh giới của workspace, kiểm tự động trong gate.
//!
//! Trình biên dịch đã lo phần chính: thứ không `pub` thì crate khác không dùng
//! được, và cargo cấm phụ thuộc vòng. File này kiểm những thứ trình biên dịch
//! KHÔNG biết (xem docs/DECISIONS.md, D-031):
//!
//! 1. `layout` — crate nằm đúng chỗ đã quy ước.
//! 2. `license`, `lints` — mọi crate khai license chung và dùng lint chung.
//!    `lint-allow` — không tắt các lint an toàn (`unsafe`, `unwrap`, `expect`,
//!    `panic`…) trong `src/`; chỉ file trong `tests/` và `benches/` được tắt.
//!    `env-access` — cấu hình chỉ đọc qua `autoxide_kernel::config`, không gọi
//!    `std::env::var` rải rác.
//! 3. `dependency` — chiều phụ thuộc: kernel không biết module, testkit chỉ dùng
//!    cho test, không ai phụ thuộc vào binary.
//! 4. `module-surface` — module chỉ mở ra ngoài `pub mod api` (hợp đồng) và
//!    `pub fn module` (để binary đăng ký). Nhờ vậy trình biên dịch tự chặn mọi
//!    truy cập vào phần bên trong, kể cả entity (bảng) của module.
//! 5. `module-import` — module khác chỉ được dùng `<crate>::api::…`.
//! 6. `api-self-contained` — `api` không kéo phần bên trong ra ngoài.
//! 7. `module-routes` — route của module đăng ký qua `.routes(routes!(…))` để luôn
//!    có tài liệu OpenAPI; không dùng `.route(…)` trần.
//! 8. `handler-database` — handler (`src/http…`) và tool MCP (`src/tools…`) không tự
//!    đọc hay ghi database: việc đó thuộc service (luật kiến trúc 5 và 8 của CLAUDE.md).
//! 9. `entity-tenant` — entity là bảng cấp khách hàng, có `tenant_id` (bước 33; ở `tenancy.rs`).
//! 10. `raw-statement` — câu SQL viết tay dựng bằng `autoxide_kernel::database::statement` (ở
//!     `raw_statement.rs`): nó thêm kiểu cho giá trị thời gian mà Postgres cần.
//!
//! Luật 4–6 đọc mã nguồn theo dòng, dựa vào việc `cargo fmt` đã chuẩn hoá định
//! dạng (gate chạy fmt trước). Đây là lưới chặn lỗi vô ý, không phải bộ phân
//! tích cú pháp đầy đủ.

use std::fmt;
use std::fs;
use std::path::Path;

use crate::workspace::{DepKind, Package, Role, Workspace};

mod raw_statement;
mod test_layout;

/// License mọi crate phải khai (D-021).
pub const LICENSE: &str = "MIT OR Apache-2.0";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// Tên luật, ổn định để tra trong file này.
    pub rule: &'static str,
    /// Crate hoặc `file:dòng`.
    pub location: String,
    pub message: String,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}: {}", self.rule, self.location, self.message)?;
        // Một dòng "cách sửa" dưới vi phạm (`hints.rs`).
        match crate::hints::fix(self.rule) {
            Some(fix) => write!(f, "\n    how to fix: {fix}"),
            None => Ok(()),
        }
    }
}

/// Kiểm toàn bộ workspace trên đĩa. `Err` là lỗi đọc file, không phải vi phạm.
pub fn check_workspace(workspace: &Workspace) -> Result<Vec<Violation>, String> {
    let mut violations = Vec::new();

    for package in &workspace.packages {
        let manifest_path = workspace.root.join(&package.dir).join("Cargo.toml");
        let manifest = read(&manifest_path)?;
        violations.extend(check_manifest(package, &manifest));
    }

    violations.extend(check_dependencies(workspace));

    for package in &workspace.packages {
        violations.extend(test_layout::check(&workspace.root, package));
        violations.extend(raw_statement::check(&workspace.root, package)?);
    }

    for package in &workspace.packages {
        let crate_dir = workspace.root.join(&package.dir);
        for file in rust_files(&crate_dir.join("src"))? {
            let text = read(&crate_dir.join(&file))?;
            violations.extend(check_lint_allows(package, &file, &text));
            violations.extend(check_env_access(package, &file, &text));
        }
    }

    let modules: Vec<&Package> =
        workspace.packages.iter().filter(|package| package.role == Role::Module).collect();
    for module in &modules {
        let other_idents: Vec<String> = modules
            .iter()
            .filter(|other| other.name != module.name)
            .map(|other| other.ident())
            .collect();
        let crate_dir = workspace.root.join(&module.dir);
        for file in rust_files(&crate_dir.join("src"))? {
            let text = read(&crate_dir.join(&file))?;
            violations.extend(check_module_source(module, &file, &text, &other_idents));
        }
    }

    Ok(violations)
}

/// Các luật của file này áp cho MỘT module nằm ngoài repo (`cargo xtask check-module`, xem
/// `external.rs`): `workspace` chỉ có module đó. Không kiểm `layout`, `license`, `lints`,
/// `dependency`, `test-binary` — quy ước của workspace này — và `module-import` (không biết các
/// module khác; trình biên dịch vẫn chặn phần riêng tư của chúng).
pub fn check_external_module(workspace: &Workspace) -> Result<Vec<Violation>, String> {
    let mut violations = Vec::new();
    for package in &workspace.packages {
        let crate_dir = workspace.root.join(&package.dir);
        violations.extend(raw_statement::check(&workspace.root, package)?);
        for file in rust_files(&crate_dir.join("src"))? {
            let text = read(&crate_dir.join(&file))?;
            violations.extend(check_lint_allows(package, &file, &text));
            violations.extend(check_env_access(package, &file, &text));
            violations.extend(check_module_source(package, &file, &text, &[]));
        }
    }
    Ok(violations)
}

/// Luật `layout`, `license`, `lints` cho một crate. `manifest` là nội dung
/// `Cargo.toml` của nó.
pub fn check_manifest(package: &Package, manifest: &str) -> Vec<Violation> {
    let mut violations = Vec::new();
    let location = format!("{}/Cargo.toml", package.dir);

    if package.role == Role::Unknown {
        violations.push(Violation {
            rule: "layout",
            location: location.clone(),
            message:
                "crate must live in crates/kernel, crates/macros, crates/host, crates/testkit, crates/app, \
                      modules/<name> or xtask"
                    .to_owned(),
        });
    }
    if package.license.as_deref() != Some(LICENSE) {
        violations.push(Violation {
            rule: "license",
            location: location.clone(),
            message: format!("add `license.workspace = true` (must be \"{LICENSE}\")"),
        });
    }
    if !inherits_workspace_lints(manifest) {
        violations.push(Violation {
            rule: "lints",
            location,
            message: "add a `[lints]` section with `workspace = true`".to_owned(),
        });
    }
    violations
}

/// Manifest có `[lints] workspace = true` (hoặc dạng viết tắt `lints.workspace = true`).
fn inherits_workspace_lints(manifest: &str) -> bool {
    let mut section = "";
    for line in manifest.lines() {
        // Bỏ chú thích cuối dòng và khoảng trắng để so sánh không phụ thuộc cách viết.
        let line = line.split('#').next().unwrap_or_default().trim();
        if line.starts_with('[') {
            section = line;
            continue;
        }
        let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
        let found = match section {
            "[lints]" => compact == "workspace=true",
            "" => compact == "lints.workspace=true",
            _ => false,
        };
        if found {
            return true;
        }
    }
    false
}

/// Các lint thực thi "Luật code" của CLAUDE.md; khớp với `[workspace.lints]`
/// trong `Cargo.toml` gốc.
const PROTECTED_LINTS: [&str; 7] = [
    "unsafe_code",
    "clippy::unwrap_used",
    "clippy::expect_used",
    "clippy::panic",
    "clippy::todo",
    "clippy::unimplemented",
    "clippy::dbg_macro",
];

/// Luật `lint-allow` cho một file trong `src/`: không được nới các lint an toàn
/// bằng `#[allow(...)]` hay `#[expect(...)]`. Cần `unwrap` trong test thì viết
/// test trong `#[cfg(test)] mod tests` (clippy tự miễn) hoặc trong `tests/`.
pub fn check_lint_allows(package: &Package, file: &str, text: &str) -> Vec<Violation> {
    let mut violations = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim_start();
        // Thuộc tính có dạng `#[allow(...)]`, `#![allow(...)]`, `#[cfg_attr(x, allow(...))]`.
        let is_attribute = line.starts_with("#[") || line.starts_with("#![");
        let relaxes = line.contains("allow(") || line.contains("expect(");
        if !(is_attribute && relaxes) {
            continue;
        }
        for lint in PROTECTED_LINTS {
            if contains_lint(line, lint) {
                violations.push(Violation {
                    rule: "lint-allow",
                    location: format!("{}/{}:{}", package.dir, file, index + 1),
                    message: format!(
                        "`{lint}` must not be relaxed in src/; return an error instead, \
                         or move test-only code into `#[cfg(test)]` or tests/"
                    ),
                });
            }
        }
    }
    violations
}

/// `line` có nhắc đúng lint `lint` không (không tính tên dài hơn chứa nó, vd
/// `clippy::panic_in_result_fn` không phải `clippy::panic`).
fn contains_lint(line: &str, lint: &str) -> bool {
    let is_ident_char = |c: char| c.is_alphanumeric() || c == '_' || c == ':';
    line.match_indices(lint).any(|(position, _)| {
        let before = line[..position].chars().next_back();
        let after = line[position + lint.len()..].chars().next();
        !before.is_some_and(is_ident_char) && !after.is_some_and(is_ident_char)
    })
}

/// File duy nhất của kernel được đọc biến môi trường của tiến trình.
const ENV_ACCESS_FILE: &str = "src/config/source.rs";

/// Luật `env-access`: kernel, binary và module đọc cấu hình qua `ConfigSource` /
/// `ConfigReader` của kernel (D-034) — để mọi biến đều được khai bằng `Setting`,
/// được kiểm lúc khởi động và có mặt trong `.env.example`. Testkit và xtask là
/// công cụ, không thuộc luật này.
pub fn check_env_access(package: &Package, file: &str, text: &str) -> Vec<Violation> {
    let governed = matches!(package.role, Role::Kernel | Role::App | Role::Host | Role::Module);
    if !governed || (package.role == Role::Kernel && file == ENV_ACCESS_FILE) {
        return Vec::new();
    }
    let mut violations = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        let reads_environment = ["env::var(", "env::var_os(", "env::vars(", "env::vars_os("]
            .iter()
            .any(|call| line.contains(call));
        if reads_environment {
            violations.push(Violation {
                rule: "env-access",
                location: format!("{}/{}:{}", package.dir, file, index + 1),
                message: "read configuration through `autoxide_kernel::config` (declare a \
                          `Setting`), not from the process environment directly"
                    .to_owned(),
            });
        }
    }
    violations
}

/// Luật `dependency`: chiều phụ thuộc giữa các crate nội bộ.
pub fn check_dependencies(workspace: &Workspace) -> Vec<Violation> {
    let mut violations = Vec::new();
    for package in &workspace.packages {
        for dep in &package.deps {
            // Chỉ xét dependency là crate của workspace; thư viện ngoài do cargo-deny lo.
            let Some(target) = workspace.package(&dep.name) else { continue };
            if let Some(reason) = forbidden_dependency(package.role, target.role, dep.kind) {
                violations.push(Violation {
                    rule: "dependency",
                    location: format!("{}/Cargo.toml", package.dir),
                    message: format!("must not depend on `{}`: {reason}", target.name),
                });
            }
        }
    }
    violations
}

/// Trả lý do nếu crate vai `from` không được phụ thuộc crate vai `to`.
fn forbidden_dependency(from: Role, to: Role, kind: DepKind) -> Option<&'static str> {
    match (from, to) {
        // Vị trí lạ đã bị luật `layout` báo; không báo chồng.
        (Role::Unknown, _) | (_, Role::Unknown) => None,
        (_, Role::App) => Some("the binary is the top of the graph; nothing depends on it"),
        (_, Role::Tool) => Some("xtask is developer tooling, not a library"),
        (Role::Tool, _) => Some("xtask must stay independent of the code it checks"),
        (_, Role::Testkit) if kind != DepKind::Dev => {
            Some("testkit is test-only; declare it under [dev-dependencies]")
        }
        (Role::Kernel | Role::Testkit | Role::Host, Role::Module) => {
            Some("kernel, testkit and host must not know any module (dependencies point inward)")
        }
        (from, Role::Host) if from != Role::App => Some(
            "autoxide-host is internal to the binary, not a contract; log with `tracing`, record with \
             `metrics`, and take everything else from `autoxide_kernel`",
        ),
        (Role::Macros, _) => Some("the macro crate depends on no workspace crate"),
        (from, Role::Macros) if from != Role::Kernel => Some(
            "use the macros through `autoxide_kernel` (`autoxide_kernel::db::Record`), not this crate",
        ),
        _ => None,
    }
}

/// Luật 4–6 cho một file `.rs` của module. `file` là đường dẫn tương đối so với
/// thư mục crate (vd `src/lib.rs`); `other_module_idents` là tên crate (dạng
/// `autoxide_xxx`) của các module KHÁC.
pub fn check_module_source(
    module: &Package,
    file: &str,
    text: &str,
    other_module_idents: &[String],
) -> Vec<Violation> {
    let mut violations = Vec::new();
    let is_lib = file == "src/lib.rs";
    let is_api = file == "src/api.rs" || file.starts_with("src/api/");
    let is_entry_point = !file.ends_with("/tests.rs")
        && !file.contains("/tests/")
        && (file == "src/http.rs"
            || file.starts_with("src/http/")
            || file == "src/tools.rs"
            || file.starts_with("src/tools/"));
    // Test nằm cuối file trong `#[cfg(test)] mod tests`: từ đó trở xuống không phải handler.
    let mut in_tests = false;

    for (index, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        in_tests = in_tests || line.trim() == "#[cfg(test)]";
        let location = format!("{}/{}:{}", module.dir, file, index + 1);
        let mut report = |rule: &'static str, message: String| {
            violations.push(Violation { rule, location: location.clone(), message });
        };

        if is_lib && is_forbidden_public_item(line) {
            report(
                "module-surface",
                "a module crate may only expose `pub mod api;` and `pub fn module(...)`; \
                 make this `pub(crate)` or move it into `api`"
                    .to_owned(),
            );
        }
        if line.contains("#[macro_export]") {
            report(
                "module-surface",
                "`#[macro_export]` exposes a macro at the crate root; modules must not export macros"
                    .to_owned(),
            );
        }
        for ident in other_module_idents {
            if uses_path_outside(line, ident, "api") {
                report(
                    "module-import",
                    format!("use another module only through its contract: `{ident}::api::...`"),
                );
            }
        }
        if let Some(call) = UNDOCUMENTED_ROUTE_CALLS.iter().find(|call| line.contains(*call)) {
            report(
                "module-routes",
                format!(
                    "`{}…)` adds a route without OpenAPI documentation; register handlers with \
                     `.routes(routes!(handler))`",
                    call.trim_start_matches('.')
                ),
            );
        }
        if is_entry_point && !in_tests && touches_database(line) {
            report(
                "handler-database",
                "handlers and MCP tools only call the service layer; move this database access \
                 into a service method"
                    .to_owned(),
            );
        }
        if is_api && uses_path_outside(line, "crate", "api") {
            report(
                "api-self-contained",
                "`api` is the published contract and must not reference crate internals; \
                 define the type in `api` and let internals depend on it"
                    .to_owned(),
            );
        }
    }
    violations
}

/// Dòng mở kết nối database để đọc hay ghi (`service.database.read()`, `begin_write()`…).
fn touches_database(line: &str) -> bool {
    line.contains("database")
        && [".read()", ".write()", "begin_write("].iter().any(|call| line.contains(call))
}

/// Các lời gọi thêm route mà KHÔNG kèm tài liệu OpenAPI. (`.routes(` — có `s` — là
/// cách đúng và không khớp các chuỗi này.)
const UNDOCUMENTED_ROUTE_CALLS: [&str; 4] =
    [".route(", ".route_service(", ".nest_service(", ".fallback("];

/// Một dòng ở cấp cao nhất của `lib.rs` (cột 0, sau khi fmt) mở thứ gì đó ra
/// ngoài mà không phải hai thứ được phép.
fn is_forbidden_public_item(line: &str) -> bool {
    // `pub(crate)` không khớp `"pub "` nên không bị tính.
    line.starts_with("pub ") && line != "pub mod api;" && !line.starts_with("pub fn module(")
}

/// `line` có dùng định danh `root` mà KHÔNG đi tiếp vào `root::<allowed>` không?
///
/// Ví dụ với `root = "autoxide_auth"`, `allowed = "api"`:
/// `autoxide_auth::api::Users` hợp lệ; `autoxide_auth::service::x`,
/// `autoxide_auth::{api, module}` và `use autoxide_auth as a` đều vi phạm.
fn uses_path_outside(line: &str, root: &str, allowed: &str) -> bool {
    let is_ident_char = |c: char| c.is_alphanumeric() || c == '_';
    let mut rest = line;
    let mut previous: Option<char> = None;

    while let Some(position) = rest.find(root) {
        let before = rest[..position].chars().next_back().or(previous);
        let after = &rest[position + root.len()..];
        // Chỉ tính khi `root` đứng riêng thành một định danh, không phải một
        // phần của tên dài hơn (vd `autoxide_auth_extra`, `my_crate`).
        let standalone =
            !before.is_some_and(is_ident_char) && !after.chars().next().is_some_and(is_ident_char);
        if standalone {
            let inside_allowed = after
                .strip_prefix("::")
                .and_then(|path| path.strip_prefix(allowed))
                .is_some_and(|tail| !tail.chars().next().is_some_and(is_ident_char));
            if !inside_allowed {
                return true;
            }
        }
        previous = root.chars().next_back();
        rest = after;
    }
    false
}

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|error| format!("cannot read {}: {error}", path.display()))
}

/// Mọi file `.rs` dưới `src_dir`, trả về dạng `src/...` dùng `/`, đã sắp xếp.
pub(crate) fn rust_files(src_dir: &Path) -> Result<Vec<String>, String> {
    let mut files = Vec::new();
    collect_rust_files(src_dir, "src", &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_rust_files(dir: &Path, prefix: &str, files: &mut Vec<String>) -> Result<(), String> {
    let entries =
        fs::read_dir(dir).map_err(|error| format!("cannot list {}: {error}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot list {}: {error}", dir.display()))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let relative = format!("{prefix}/{name}");
        if path.is_dir() {
            collect_rust_files(&path, &relative, files)?;
        } else if name.ends_with(".rs") {
            files.push(relative);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::Dep;

    const GOOD_MANIFEST: &str = "[package]\nname = \"x\"\n\n[lints]\nworkspace = true\n";

    fn package(name: &str, dir: &str, deps: &[(&str, DepKind)]) -> Package {
        Package {
            name: name.to_owned(),
            license: Some(LICENSE.to_owned()),
            dir: dir.to_owned(),
            role: Role::from_dir(dir),
            deps: deps
                .iter()
                .map(|(name, kind)| Dep { name: (*name).to_owned(), kind: *kind })
                .collect(),
        }
    }

    /// Workspace mẫu đủ các vai; `extra` thêm dependency cho từng crate theo tên.
    fn workspace(extra: &[(&str, &str, DepKind)]) -> Workspace {
        let layout = [
            ("autoxide-kernel", "crates/kernel"),
            ("autoxide-testkit", "crates/testkit"),
            ("autoxide-host", "crates/host"),
            ("autoxide", "crates/app"),
            ("autoxide-auth", "modules/auth"),
            ("autoxide-audit", "modules/audit"),
            ("xtask", "xtask"),
        ];
        let packages = layout
            .iter()
            .map(|(name, dir)| {
                let deps: Vec<(&str, DepKind)> = extra
                    .iter()
                    .filter(|(from, _, _)| from == name)
                    .map(|(_, to, kind)| (*to, *kind))
                    .collect();
                package(name, dir, &deps)
            })
            .collect();
        Workspace { root: "/repo".into(), packages }
    }

    fn rules(violations: &[Violation]) -> Vec<&'static str> {
        violations.iter().map(|violation| violation.rule).collect()
    }

    // --- manifest ---

    #[test]
    fn manifest_dung_thi_khong_vi_pham() {
        let package = package("autoxide-kernel", "crates/kernel", &[]);
        assert_eq!(check_manifest(&package, GOOD_MANIFEST), []);
    }

    #[test]
    fn crate_o_vi_tri_la_bi_bao() {
        let package = package("stray", "libs/stray", &[]);
        assert_eq!(rules(&check_manifest(&package, GOOD_MANIFEST)), ["layout"]);
    }

    #[test]
    fn thieu_hoac_sai_license_bi_bao() {
        let mut package = package("autoxide-kernel", "crates/kernel", &[]);
        package.license = None;
        assert_eq!(rules(&check_manifest(&package, GOOD_MANIFEST)), ["license"]);
        package.license = Some("MIT".to_owned());
        assert_eq!(rules(&check_manifest(&package, GOOD_MANIFEST)), ["license"]);
    }

    #[test]
    fn thieu_lints_workspace_bi_bao() {
        let package = package("autoxide-kernel", "crates/kernel", &[]);
        for manifest in [
            "[package]\nname = \"x\"\n",
            // `workspace = true` của section khác không được tính.
            "[package]\nversion.workspace = true\n\n[dependencies]\nserde = { workspace = true }\n",
            "[lints]\n# workspace = true\n",
            "[lints.clippy]\nunwrap_used = \"deny\"\n",
        ] {
            assert_eq!(rules(&check_manifest(&package, manifest)), ["lints"], "{manifest}");
        }
    }

    #[test]
    fn cac_cach_viet_lints_workspace_deu_duoc_nhan() {
        for manifest in [
            "[lints]\nworkspace = true\n",
            "[lints]\nworkspace=true # dùng lint chung\n",
            "lints.workspace = true\n\n[package]\nname = \"x\"\n",
        ] {
            assert!(inherits_workspace_lints(manifest), "{manifest}");
        }
    }

    // --- dependency ---

    #[test]
    fn chieu_phu_thuoc_hop_le() {
        let workspace = workspace(&[
            ("autoxide-kernel", "autoxide-testkit", DepKind::Dev),
            ("autoxide-testkit", "autoxide-kernel", DepKind::Normal),
            ("autoxide-auth", "autoxide-kernel", DepKind::Normal),
            ("autoxide-auth", "autoxide-testkit", DepKind::Dev),
            ("autoxide-audit", "autoxide-auth", DepKind::Normal),
            ("autoxide", "autoxide-kernel", DepKind::Normal),
            ("autoxide", "autoxide-auth", DepKind::Normal),
            ("autoxide", "autoxide-audit", DepKind::Normal),
            ("autoxide", "autoxide-testkit", DepKind::Dev),
            ("autoxide-host", "autoxide-kernel", DepKind::Normal),
            ("autoxide-host", "autoxide-testkit", DepKind::Dev),
            ("autoxide", "autoxide-host", DepKind::Normal),
            // Thư viện ngoài không thuộc phạm vi luật này.
            ("xtask", "serde", DepKind::Normal),
        ]);
        assert_eq!(check_dependencies(&workspace), []);
    }

    #[test]
    fn chieu_phu_thuoc_bi_cam() {
        let forbidden = [
            ("autoxide-kernel", "autoxide-auth", DepKind::Normal),
            ("autoxide-kernel", "autoxide-auth", DepKind::Dev),
            ("autoxide-testkit", "autoxide-auth", DepKind::Normal),
            ("autoxide-auth", "autoxide", DepKind::Normal),
            ("autoxide-auth", "autoxide-testkit", DepKind::Normal),
            ("autoxide-auth", "autoxide-testkit", DepKind::Build),
            ("autoxide", "autoxide-testkit", DepKind::Normal),
            ("autoxide", "xtask", DepKind::Dev),
            ("xtask", "autoxide-kernel", DepKind::Normal),
            // Chỉ binary dùng `autoxide-host` (D-20261128-host-crate).
            ("autoxide-auth", "autoxide-host", DepKind::Normal),
            ("autoxide-auth", "autoxide-host", DepKind::Dev),
            ("autoxide-kernel", "autoxide-host", DepKind::Dev),
            ("autoxide-testkit", "autoxide-host", DepKind::Normal),
            ("autoxide-host", "autoxide-auth", DepKind::Normal),
        ];
        for case in forbidden {
            let violations = check_dependencies(&workspace(&[case]));
            assert_eq!(rules(&violations), ["dependency"], "{case:?}");
            assert_eq!(violations[0].location, {
                let (from, _, _) = case;
                let workspace = workspace(&[]);
                format!("{}/Cargo.toml", workspace.package(from).unwrap().dir)
            });
        }
    }

    // --- lint-allow ---

    #[test]
    fn noi_lint_an_toan_trong_src_bi_bao() {
        let package = package("autoxide-kernel", "crates/kernel", &[]);
        for line in [
            "#![allow(clippy::unwrap_used)]",
            "#[allow(clippy::expect_used)]",
            "    #[allow(clippy::panic)]",
            "#[allow(unsafe_code)]",
            "#[expect(clippy::unwrap_used, reason = \"x\")]",
            "#[cfg_attr(not(test), allow(clippy::todo))]",
            "#[allow(dead_code, clippy::dbg_macro)]",
            "#[allow(clippy::unimplemented)]",
        ] {
            let violations = check_lint_allows(&package, "src/lib.rs", &format!("\n{line}\n"));
            assert_eq!(rules(&violations), ["lint-allow"], "{line}");
            assert_eq!(violations[0].location, "crates/kernel/src/lib.rs:2");
        }
        // Một dòng nới nhiều lint thì báo từng lint.
        let both = "#![allow(clippy::unwrap_used, clippy::expect_used)]";
        assert_eq!(check_lint_allows(&package, "src/lib.rs", both).len(), 2);
    }

    #[test]
    fn lint_khac_va_code_thuong_khong_bi_bao() {
        let package = package("autoxide-kernel", "crates/kernel", &[]);
        let text = "\
#[allow(dead_code)]
#[allow(clippy::too_many_arguments)]
#[allow(clippy::panic_in_result_fn)]
#[deny(clippy::unwrap_used)]
// #[allow(clippy::unwrap_used)] chỉ là chú thích
let message = \"do not allow(clippy::unwrap_used) here\";
let value = result.expect(\"clippy::expect_used bắt dòng này, không phải xtask\");
";
        assert_eq!(check_lint_allows(&package, "src/lib.rs", text), []);
    }

    // --- env-access ---

    #[test]
    fn doc_bien_moi_truong_truc_tiep_bi_bao() {
        for (name, dir) in [
            ("autoxide-kernel", "crates/kernel"),
            ("autoxide", "crates/app"),
            ("autoxide-auth", "modules/auth"),
        ] {
            let package = package(name, dir, &[]);
            for line in [
                "let url = std::env::var(\"DATABASE_URL\")?;",
                "    if env::var_os(\"DEBUG\").is_some() {",
                "for (name, value) in std::env::vars() {",
                "for (name, value) in std::env::vars_os() {",
            ] {
                let violations = check_env_access(&package, "src/service.rs", line);
                assert_eq!(rules(&violations), ["env-access"], "{name}: {line}");
            }
        }
    }

    #[test]
    fn noi_duoc_phep_doc_bien_moi_truong() {
        let line = "for (name, value) in std::env::vars_os() {";
        // File nguồn cấu hình của kernel.
        let kernel = package("autoxide-kernel", "crates/kernel", &[]);
        assert_eq!(check_env_access(&kernel, "src/config/source.rs", line), []);
        // Cùng tên file ở crate khác thì không được miễn.
        let app = package("autoxide", "crates/app", &[]);
        assert_eq!(rules(&check_env_access(&app, "src/config/source.rs", line)), ["env-access"]);
        // Công cụ: testkit đọc TEST_DATABASE_URL, xtask đọc CARGO.
        for (name, dir) in [("autoxide-testkit", "crates/testkit"), ("xtask", "xtask")] {
            assert_eq!(check_env_access(&package(name, dir, &[]), "src/lib.rs", line), []);
        }
    }

    #[test]
    fn code_khong_doc_moi_truong_thi_khong_bi_bao() {
        let kernel = package("autoxide-kernel", "crates/kernel", &[]);
        let text = "\
// std::env::var(\"X\") trong chú thích
let directory = std::env::temp_dir();
let arguments: Vec<String> = std::env::args().collect();
let version = env!(\"CARGO_PKG_VERSION\");
";
        assert_eq!(check_env_access(&kernel, "src/lib.rs", text), []);
    }

    // --- module source ---

    fn auth() -> Package {
        package("autoxide-auth", "modules/auth", &[])
    }

    fn check(file: &str, text: &str) -> Vec<Violation> {
        check_module_source(&auth(), file, text, &["autoxide_audit".to_owned()])
    }

    #[test]
    fn handler_va_tool_goi_database_bi_bao() {
        for (file, line) in [
            ("src/http.rs", "    let read = service.database.read();"),
            (
                "src/http/entries.rs",
                "    let rows = Entity::find().all(state.database.read()).await?;",
            ),
            ("src/tools.rs", "    let transaction = service.database.begin_write().await?;"),
            ("src/tools/notes.rs", "    let connection = self.database.write();"),
        ] {
            assert_eq!(rules(&check(file, line)), ["handler-database"], "{file}: {line}");
        }
    }

    #[test]
    fn database_trong_service_va_test_khong_bi_bao() {
        let line = "    let read = self.database.read();";
        for file in ["src/service.rs", "src/http/tests.rs", "src/tools/tests/mod.rs"] {
            assert_eq!(check(file, line), [], "{file}");
        }
        // Test ở cuối file handler, và dòng chỉ nhắc tới database mà không mở kết nối.
        let text =
            "fn handler() {}\n\n#[cfg(test)]\nmod tests {\n    let r = app.database.read();\n}\n";
        assert_eq!(check("src/http.rs", text), []);
        assert_eq!(
            check("src/http.rs", "    let events = recorded_events(&services.database).await;"),
            []
        );
    }

    #[test]
    fn lib_rs_dung_khuon_thi_khong_vi_pham() {
        let text = "\
//! Module auth.

pub mod api;
mod entity;
pub(crate) mod service;

pub fn module() -> AuthModule {
    AuthModule
}

struct AuthModule;

impl AuthModule {
    pub fn name(&self) -> &'static str {
        \"auth\"
    }
}
";
        assert_eq!(check("src/lib.rs", text), []);
    }

    #[test]
    fn lib_rs_mo_thua_ra_ngoai_bi_bao() {
        for line in [
            "pub mod entity;",
            "pub mod service;",
            "pub use entity::user::Model;",
            "pub struct AuthModule;",
            "pub fn helper() {}",
            "pub mod api {",
            "pub const X: u8 = 1;",
        ] {
            let violations = check("src/lib.rs", &format!("mod a;\n{line}\n"));
            assert_eq!(rules(&violations), ["module-surface"], "{line}");
            assert_eq!(violations[0].location, "modules/auth/src/lib.rs:2");
        }
    }

    #[test]
    fn pub_o_file_khac_lib_rs_khong_thuoc_luat_be_mat() {
        // `pub` trong một module riêng tư (`mod service;`) không lọt ra ngoài crate.
        assert_eq!(check("src/service.rs", "pub struct Service;\npub fn run() {}\n"), []);
    }

    #[test]
    fn macro_export_bi_bao_o_moi_file() {
        let text = "#[macro_export]\nmacro_rules! m { () => {}; }\n";
        assert_eq!(rules(&check("src/util.rs", text)), ["module-surface"]);
    }

    #[test]
    fn route_khong_co_tai_lieu_trong_module_bi_bao() {
        for line in [
            "        .route(\"/export\", get(export))",
            "let router = OpenApiRouter::new().route(\"/x\", post(create));",
            "    .route_service(\"/files\", files)",
            "    .nest_service(\"/static\", files)",
            "    .fallback(not_found)",
        ] {
            assert_eq!(rules(&check("src/routes.rs", line)), ["module-routes"], "{line}");
        }
    }

    #[test]
    fn route_dang_ky_kem_tai_lieu_thi_duoc() {
        let text = "\
OpenApiRouter::new()
    .routes(routes!(list_notes, create_note))
    .routes(routes!(get_note))
    .nest(\"/drafts\", drafts::routes())
    .with_state(state)
// .route(\"/x\", get(x)) trong chú thích thì không sao
";
        assert_eq!(check("src/routes.rs", text), []);
    }

    #[test]
    fn dung_module_khac_qua_api_thi_duoc() {
        let text = "\
use autoxide_audit::api::AuditLog;
use autoxide_audit::api::{AuditLog, Entry};
use autoxide_audit::api;
fn f(log: &dyn autoxide_audit::api::AuditLog) {}
// Chú thích nhắc tới autoxide_audit::service thì không sao.
use autoxide_audit_extra::anything;
use my_autoxide_audit::anything;
";
        assert_eq!(check("src/service.rs", text), []);
    }

    #[test]
    fn dung_phan_ben_trong_cua_module_khac_bi_bao() {
        for line in [
            "use autoxide_audit::service::write;",
            "use autoxide_audit::entity::audit_entry::Entity;",
            "use autoxide_audit::{api::AuditLog, module};",
            "use autoxide_audit::module;",
            "use autoxide_audit::apiary::x;",
            "use autoxide_audit as audit;",
            "use autoxide_audit;",
            "let m = autoxide_audit::module();",
            // Hợp lệ đứng trước không che được vi phạm đứng sau trên cùng dòng.
            "f(autoxide_audit::api::X, autoxide_audit::service::Y);",
        ] {
            assert_eq!(rules(&check("src/service.rs", line)), ["module-import"], "{line}");
        }
    }

    #[test]
    fn api_tu_dung_duoc_thi_khong_vi_pham() {
        let text = "\
use crate::api::types::UserId;
use crate::api::{self, types};
use autoxide_kernel::Error;

pub trait Users {
    fn find(&self, id: UserId) -> Option<crate::api::types::User>;
}
";
        assert_eq!(check("src/api.rs", text), []);
        assert_eq!(check("src/api/users.rs", text), []);
    }

    #[test]
    fn api_keo_phan_ben_trong_ra_ngoai_bi_bao() {
        for line in [
            "pub use crate::entity::user::Model as User;",
            "use crate::service::Service;",
            "pub type Row = crate::entity::user::Model;",
            "use crate::{api::types, entity};",
            "use crate::apis::x;",
        ] {
            assert_eq!(rules(&check("src/api.rs", line)), ["api-self-contained"], "{line}");
            assert_eq!(rules(&check("src/api/users.rs", line)), ["api-self-contained"], "{line}");
        }
    }

    #[test]
    fn ngoai_api_thi_duoc_dung_crate_tu_do() {
        assert_eq!(check("src/service.rs", "use crate::entity::user;\n"), []);
        // Tên file chỉ bắt đầu bằng "api" không phải là hợp đồng.
        assert_eq!(check("src/api_keys.rs", "use crate::entity::api_key;\n"), []);
    }

    #[test]
    fn vi_pham_in_ra_co_ten_luat_va_vi_tri() {
        let violations = check("src/lib.rs", "pub mod entity;\n");
        assert!(
            violations[0]
                .to_string()
                .starts_with("[module-surface] modules/auth/src/lib.rs:1: a module crate may only")
        );
    }

    // --- đọc từ đĩa ---

    #[test]
    fn kiem_workspace_that_tren_dia() {
        // Dựng một workspace nhỏ trong thư mục tạm: một module đúng, một module sai.
        let root = std::env::temp_dir().join(format!("autoxide-xtask-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let write = |path: &str, text: &str| {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        write("modules/auth/Cargo.toml", GOOD_MANIFEST);
        write("modules/auth/src/lib.rs", "pub mod api;\nmod service;\n");
        write("modules/auth/src/api.rs", "pub trait Users {}\n");
        write("modules/auth/src/service.rs", "use autoxide_audit::api::AuditLog;\n");
        write("modules/audit/Cargo.toml", "[package]\nname = \"autoxide-audit\"\n");
        write("modules/audit/src/lib.rs", "pub mod api;\npub mod entity;\n");
        write("modules/audit/src/api/mod.rs", "pub use crate::entity::Row;\n");
        write(
            "modules/audit/src/entity.rs",
            "#![allow(clippy::unwrap_used)]\nuse autoxide_auth::service::Service;\n",
        );
        write("modules/audit/src/notes.txt", "pub mod ignored;\n");

        let workspace = Workspace {
            root: root.clone(),
            packages: vec![
                package("autoxide-audit", "modules/audit", &[]),
                package("autoxide-auth", "modules/auth", &[]),
            ],
        };
        let violations = check_workspace(&workspace).unwrap();
        fs::remove_dir_all(&root).unwrap();

        let found: Vec<(&str, &str)> = violations
            .iter()
            .map(|violation| (violation.rule, violation.location.as_str()))
            .collect();
        assert_eq!(
            found,
            [
                ("lints", "modules/audit/Cargo.toml"),
                ("lint-allow", "modules/audit/src/entity.rs:1"),
                ("api-self-contained", "modules/audit/src/api/mod.rs:1"),
                ("module-import", "modules/audit/src/entity.rs:2"),
                ("module-surface", "modules/audit/src/lib.rs:2"),
            ]
        );
    }

    #[test]
    fn thieu_file_la_loi_chu_khong_phai_vi_pham() {
        let workspace = Workspace {
            root: "/nonexistent-autoxide-root".into(),
            packages: vec![package("autoxide-kernel", "crates/kernel", &[])],
        };
        assert!(check_workspace(&workspace).unwrap_err().contains("cannot read"));
    }
}
