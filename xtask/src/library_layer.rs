//! Luật của lớp kernel thay thư viện (D-20261126-library-isolation, D-20261126-kernel-data-layer):
//!
//! - `wrapped-libraries`: module không khai thư viện đã có lớp của kernel (danh sách ở [`LIBRARIES`]). Module chưa
//!   chuyển xong nằm trong [`BASELINE`], chỉ được giảm.
//! - `private-paths`: mã viết tay không gọi `autoxide_kernel::__private` — đường đó chỉ cho mã do macro của kernel
//!   sinh ra, không thuộc hợp đồng.
//! - `raw-sql`: số chỗ gọi `.sql(` (SQL viết tay của lớp `db`) trong mã không phải test của module, theo
//!   [`BASELINE`] chỉ được giảm: kiểm `:tenant` của nó chỉ là rào phụ, nên mỗi chỗ mới là một quyết định có lý do.

use std::fs;
use std::path::Path;

use crate::boundaries::{Violation, rust_files};
use crate::workspace::{DepKind, Role, Workspace};

/// Thư viện đã có (hay sẽ có) lớp của kernel; module không khai trực tiếp.
pub const LIBRARIES: &str = "xtask/wrapped-libraries.txt";
/// Chỗ cũ chờ chuyển của `wrapped-libraries` và `raw-sql` (cùng định dạng `security-baseline.txt`).
pub const BASELINE: &str = "xtask/library-baseline.txt";

/// Thư viện trong [`LIBRARIES`]: một tên mỗi dòng, `#` là chú thích.
fn libraries(root: &Path) -> Result<Vec<String>, String> {
    let text = fs::read_to_string(root.join(LIBRARIES))
        .map_err(|error| format!("{LIBRARIES}: {error}"))?;
    Ok(text
        .lines()
        .map(|line| line.split('#').next().unwrap_or_default().trim().to_owned())
        .filter(|line| !line.is_empty())
        .collect())
}

/// Vi phạm có danh sách chỗ cũ ([`BASELINE`]): `wrapped-libraries`, `raw-sql`, `raw-database`.
pub fn check(workspace: &Workspace) -> Result<Vec<Violation>, String> {
    let libraries = libraries(&workspace.root)?;
    let mut violations = Vec::new();
    for package in workspace.packages.iter().filter(|package| package.role == Role::Module) {
        let manifest = format!("{}/Cargo.toml", package.dir);
        // `tower` trong test của module được phép (thiết kế §1.1): chỉ ở `[dev-dependencies]`.
        let allowed_in_dev =
            |dep: &crate::workspace::Dep| dep.kind == DepKind::Dev && dep.name == "tower";
        for dep in
            package.deps.iter().filter(|dep| libraries.contains(&dep.name) && !allowed_in_dev(dep))
        {
            // Phụ thuộc của test / bench tính riêng: testkit còn lộ kiểu `http` (`StatusCode`, `Method`), nên test của
            // module đã chuyển vẫn cần `axum` tới khi testkit bọc chúng; baseline riêng, cũng chỉ giảm.
            let (rule, section) = match dep.kind {
                DepKind::Dev => ("wrapped-libraries-dev", "[dev-dependencies]"),
                DepKind::Build => ("wrapped-libraries", "[build-dependencies]"),
                DepKind::Normal => ("wrapped-libraries", "[dependencies]"),
            };
            violations.push(Violation {
                rule,
                location: manifest.clone(),
                message: format!(
                    "module depends on `{}` ({section}), which the kernel wraps: use the kernel layer instead \
                     (`autoxide_kernel::db` for queries, `autoxide_kernel::http` for endpoints; see \
                     docs/research/kernel-contract-1-0.md) and remove the dependency",
                    dep.name
                ),
            });
        }
        for (file, text) in module_files(workspace, package)? {
            if is_test_file(&file) {
                continue;
            }
            for (index, line) in code_lines(&text) {
                let location = format!("{}/{file}:{}", package.dir, index + 1);
                if line.contains(".sql(") {
                    violations.push(Violation {
                        rule: "raw-sql",
                        location: location.clone(),
                        message: "hand-written SQL: express it with the `db` query builder; if it is measured to \
                                  be slow, record the reason (skill `record-decision`) and add a tenant-isolation \
                                  test"
                            .to_owned(),
                    });
                }
                for _ in 0..line.matches(".database()").count() {
                    violations.push(Violation {
                        rule: "raw-database",
                        location: location.clone(),
                        message: "the raw database (`TenantContext::database()`) skips the tenant filter: query \
                                  through `tenant.db()` (control-plane tables: `context.control_plane()`)"
                            .to_owned(),
                    });
                }
            }
        }
    }
    Ok(violations)
}

/// Luật không có danh sách chỗ cũ, cho module đã chuyển và cho mọi crate:
///
/// - `record-registered`: mọi `#[derive(Record)]` trong mã không phải test của module có mặt trong
///   `Module::records()` (`RecordInfo::of::<Tên>()`), để bước đối chiếu lúc khởi động thấy nó.
/// - `wrapped-paths`: module đã bỏ hết thư viện bọc (không còn dòng `wrapped-libraries`) chỉ được gọi `sea_orm` và
///   `migration::prelude` trong `src/migration/` — cầu `ConnectionTrait` tạm thời của handle không thành lối đi vòng.
/// - `testing-feature`: chỉ `crates/testkit` bật feature `testing` của kernel.
pub fn check_strict(workspace: &Workspace) -> Result<Vec<Violation>, String> {
    let libraries = libraries(&workspace.root)?;
    let mut violations = Vec::new();
    // Manifest gốc (`[workspace.dependencies]`) và manifest của mọi crate trừ testkit.
    let mut manifests = vec!["Cargo.toml".to_owned()];
    manifests.extend(
        workspace
            .packages
            .iter()
            .filter(|package| package.role != Role::Testkit)
            .map(|package| format!("{}/Cargo.toml", package.dir)),
    );
    for manifest in manifests {
        let text = fs::read_to_string(workspace.root.join(&manifest))
            .map_err(|error| format!("{manifest}: {error}"))?;
        for line in kernel_testing_lines(&text) {
            violations.push(Violation {
                rule: "testing-feature",
                location: format!("{manifest}:{line}"),
                message: "only crates/testkit turns on the kernel feature `testing` (direct table access for \
                          tests); depend on `autoxide-testkit` under [dev-dependencies] instead"
                    .to_owned(),
            });
        }
    }
    for package in workspace.packages.iter().filter(|package| package.role == Role::Module) {
        let converted = !package
            .deps
            .iter()
            .any(|dep| dep.kind != DepKind::Dev && libraries.contains(&dep.name));
        let files = module_files(workspace, package)?;
        if converted {
            for (file, text) in &files {
                if file.starts_with("src/migration/") || file == "src/migration.rs" {
                    continue;
                }
                let syntax = syn::parse_file(text)
                    .map_err(|error| format!("{}/{file}: {error}", package.dir))?;
                for path in migration_uses(&syntax.items) {
                    violations.push(Violation {
                        rule: "wrapped-paths",
                        location: format!("{}/{file}", package.dir),
                        message: format!(
                            "`use {path}` reaches the database library through the migration prelude (or an alias of \
                             the migration module): import `MigrationTrait` from `autoxide_kernel::migration` and \
                             query through `autoxide_kernel::db`; only src/migration/ uses the prelude"
                        ),
                    });
                }
                for (index, word) in banned_words(text) {
                    violations.push(Violation {
                        rule: "wrapped-paths",
                        location: format!("{}/{file}:{}", package.dir, index + 1),
                        message: format!(
                            "`{word}` reaches the database library past the `db` layer (no tenant filter): query \
                             through `autoxide_kernel::db`; migrations under src/migration/ may use it"
                        ),
                    });
                }
            }
        }
        let mut derived = Vec::new();
        let mut registered = Vec::new();
        for (file, text) in files.iter().filter(|(file, _)| !is_test_file(file)) {
            let syntax = syn::parse_file(text)
                .map_err(|error| format!("{}/{file}: {error}", package.dir))?;
            collect_records(&syntax.items, &module_path(file), file, &mut derived);
            registered
                .extend(registered_paths(&quote::ToTokens::to_token_stream(&syntax).to_string()));
        }
        for (path, file) in &derived {
            if !is_registered(path, &registered, &derived) {
                let name = path.join("::");
                violations.push(Violation {
                    rule: "record-registered",
                    location: format!("{}/{file}", package.dir),
                    message: format!(
                        "record `{name}` is not declared in `Module::records()`: add \
                         `RecordInfo::of::<{name}>()` so the startup check compares it with the table"
                    ),
                });
            }
        }
    }
    Ok(violations)
}

/// Từ cấm trong mã của module đã chuyển (ngoài migration): đường tới thư viện database và các hàm thô của nó. Đường
/// qua `migration::prelude` (cả nhóm `migration::{prelude::…}` và bí danh `migration as m`) bắt bằng cây `use`
/// ([`migration_uses`]).
const BANNED_WORDS: &[&str] = &[
    "sea_orm",
    "sea_query",
    "ConnectionTrait",
    "execute_unprepared",
    "execute_raw",
    "query_all_raw",
    "query_one_raw",
];

/// Câu `use` (kể cả trong `mod` lồng) đi vào `migration::prelude` hay đặt bí danh cho mô-đun `migration`.
fn migration_uses(items: &[syn::Item]) -> Vec<String> {
    let mut found = Vec::new();
    for item in items {
        match item {
            syn::Item::Use(item) => {
                let text: String = quote::ToTokens::to_token_stream(&item.tree)
                    .to_string()
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect();
                let words: Vec<&str> =
                    text.split(|c: char| !c.is_alphanumeric() && c != '_').collect();
                let reaches_prelude = words.contains(&"migration") && words.contains(&"prelude");
                let aliases_migration = text.contains("migrationas");
                if reaches_prelude || aliases_migration {
                    found.push(text);
                }
            }
            syn::Item::Mod(module) => {
                if let Some((_, inner)) = &module.content {
                    found.extend(migration_uses(inner));
                }
            }
            _ => {}
        }
    }
    found
}

/// (số dòng, từ) của mọi từ cấm, so theo cả từ (không khớp một phần của tên khác), bỏ dòng chú thích.
fn banned_words(text: &str) -> Vec<(usize, &'static str)> {
    let mut found = Vec::new();
    for (index, line) in code_lines(text) {
        for word in line.split(|c: char| !c.is_alphanumeric() && c != '_') {
            if let Some(banned) = BANNED_WORDS.iter().find(|banned| **banned == word) {
                found.push((index, *banned));
            }
        }
    }
    found
}

/// Đường dẫn module của file: `src/lib.rs` → `[]`, `src/entity.rs` → `[entity]`, `src/a/mod.rs` → `[a]`.
fn module_path(file: &str) -> Vec<String> {
    let inner = file.trim_start_matches("src/").trim_end_matches(".rs");
    let mut parts: Vec<String> = inner.split('/').map(str::to_owned).collect();
    if matches!(parts.last().map(String::as_str), Some("lib" | "main" | "mod")) {
        parts.pop();
    }
    parts
}

/// Mọi struct có `derive(Record)` (kể cả trong `cfg_attr`, viết nhiều dòng, mọi mức `pub`), với đường dẫn đầy đủ.
fn collect_records(
    items: &[syn::Item],
    path: &[String],
    file: &str,
    out: &mut Vec<(Vec<String>, String)>,
) {
    for item in items {
        match item {
            syn::Item::Struct(item) if derives_record(&item.attrs) => {
                let mut full = path.to_vec();
                full.push(item.ident.to_string());
                out.push((full, file.to_owned()));
            }
            syn::Item::Mod(module) => {
                if let Some((_, inner)) = &module.content {
                    let mut inner_path = path.to_vec();
                    inner_path.push(module.ident.to_string());
                    collect_records(inner, &inner_path, file, out);
                }
            }
            _ => {}
        }
    }
}

/// `#[derive(… Record …)]` hay `#[cfg_attr(…, derive(… Record …))]`, đường dẫn đầy đủ hay ngắn.
fn derives_record(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        let name = attr.path().segments.last().map(|segment| segment.ident.to_string());
        if !matches!(name.as_deref(), Some("derive" | "cfg_attr")) {
            return false;
        }
        let tokens = quote::ToTokens::to_token_stream(attr).to_string();
        (name.as_deref() == Some("derive") || tokens.contains("derive"))
            && tokens.split(|c: char| !c.is_alphanumeric() && c != '_').any(|word| word == "Record")
    })
}

/// Đường dẫn trong `RecordInfo::of::<…>()` của chuỗi token, bỏ `crate` / `self` / `super` ở đầu.
fn registered_paths(tokens: &str) -> Vec<Vec<String>> {
    // Cách in token có lúc liền (`::<`), có lúc cách (`:: <`): bỏ mọi khoảng trắng trước khi tìm.
    let compact: String = tokens.chars().filter(|c| !c.is_whitespace()).collect();
    compact
        .split("RecordInfo::of::<")
        .skip(1)
        .map(|part| {
            part.split('>')
                .next()
                .unwrap_or_default()
                .split("::")
                .map(|segment| segment.trim().to_owned())
                .filter(|segment| {
                    !segment.is_empty() && !matches!(segment.as_str(), "crate" | "self" | "super")
                })
                .collect()
        })
        .collect()
}

/// Record `path` đã đăng ký: một đường dẫn đăng ký là đuôi của nó (so từng đoạn). Đường dẫn một đoạn (`Note`, qua
/// `use`) chỉ được tính khi không có record khác cùng tên.
fn is_registered(
    path: &[String],
    registered: &[Vec<String>],
    derived: &[(Vec<String>, String)],
) -> bool {
    registered.iter().any(|candidate| {
        if candidate.is_empty() || candidate.len() > path.len() || !path.ends_with(candidate) {
            return false;
        }
        candidate.len() > 1
            || derived.iter().filter(|(other, _)| other.last() == path.last()).count() == 1
    })
}

/// Số dòng (từ 1) bật feature `testing` của `autoxide-kernel`: bảng `[…dependencies.autoxide-kernel]` có `features`
/// chứa `"testing"`, hay bảng nội dòng `autoxide-kernel = { … "testing" … }` (kể cả kéo sang nhiều dòng).
fn kernel_testing_lines(manifest: &str) -> Vec<usize> {
    let mut found = Vec::new();
    let mut in_kernel_table = false;
    let mut inline: Option<(usize, String)> = None;
    for (index, raw) in manifest.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or_default().trim();
        if let Some((start, buffer)) = inline.as_mut() {
            buffer.push_str(line);
            if line.contains('}') {
                if buffer.contains("\"testing\"") {
                    found.push(*start + 1);
                }
                inline = None;
            }
            continue;
        }
        if line.starts_with('[') {
            in_kernel_table = line
                .trim_matches(|c| c == '[' || c == ']')
                .ends_with("dependencies.autoxide-kernel");
            continue;
        }
        if in_kernel_table {
            if line.starts_with("features") && line.contains("\"testing\"") {
                found.push(index + 1);
            }
            continue;
        }
        if line.starts_with("autoxide-kernel") && line.contains('{') {
            if line.contains('}') {
                if line.contains("\"testing\"") {
                    found.push(index + 1);
                }
            } else {
                inline = Some((index, line.to_owned()));
            }
        }
    }
    found
}

/// Các file `.rs` dưới `src/` của crate, kèm nội dung.
fn module_files(
    workspace: &Workspace,
    package: &crate::workspace::Package,
) -> Result<Vec<(String, String)>, String> {
    let src = workspace.root.join(&package.dir).join("src");
    let mut files = Vec::new();
    for file in rust_files(&src)? {
        let text = fs::read_to_string(workspace.root.join(&package.dir).join(&file))
            .map_err(|error| format!("{file}: {error}"))?;
        files.push((file, text));
    }
    Ok(files)
}

/// Dòng mã (bỏ dòng chú thích `//`), kèm số thứ tự.
fn code_lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.lines().enumerate().filter(|(_, line)| !line.trim_start().starts_with("//"))
}

/// `private-paths`: `__private`, các hàm dựng ẩn của lớp HTTP (`__add`, `__new`, `__describe`, `__filters`) và `impl
/// Endpoint for` viết tay, trong mã của mọi crate trừ kernel (nơi khai chúng) và crate macro (nơi sinh mã gọi chúng);
/// `check-module` áp cho module ngoài repo. Không có danh sách chỗ cũ.
pub fn check_private_paths(workspace: &Workspace) -> Result<Vec<Violation>, String> {
    let mut violations = Vec::new();
    for package in &workspace.packages {
        if matches!(package.role, Role::Kernel | Role::Macros | Role::Tool) {
            continue;
        }
        let dir = workspace.root.join(&package.dir);
        for sub in ["src", "tests", "benches"] {
            let base = dir.join(sub);
            if !base.is_dir() {
                continue;
            }
            for file in rust_files(&base)? {
                // `rust_files` trả đường dẫn dạng `src/…` dù thư mục gốc là gì.
                let path = base.join(file.trim_start_matches("src/"));
                let text = fs::read_to_string(&path)
                    .map_err(|error| format!("{}: {error}", path.display()))?;
                for (index, message) in private_uses(&text) {
                    violations.push(Violation {
                        rule: "private-paths",
                        location: format!(
                            "{}/{sub}/{}:{}",
                            package.dir,
                            file.trim_start_matches("src/"),
                            index + 1
                        ),
                        message,
                    });
                }
            }
        }
    }
    Ok(violations)
}

/// Hàm ẩn mà chỉ mã do macro của kernel sinh được gọi: viết tay thì dựng được route bỏ qua `#[endpoint]` (không khai ai
/// được gọi, không có trong tài liệu mà permission gate dựa vào).
const GENERATED_ONLY: &[&str] = &["__add", "__new", "__describe", "__filters"];

/// (số dòng, lời báo) của mỗi chỗ dùng đường riêng của kernel trong `text`, bỏ dòng chú thích.
fn private_uses(text: &str) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    for (index, line) in code_lines(text) {
        let words: Vec<&str> = line.split(|c: char| !c.is_alphanumeric() && c != '_').collect();
        if line.contains("__private") {
            found.push((
                index,
                "`autoxide_kernel::__private` is for code the kernel's macros generate, not part of the contract: \
                 use the public path"
                    .to_owned(),
            ));
        }
        if let Some(name) = GENERATED_ONLY.iter().find(|name| words.contains(name)) {
            found.push((
                index,
                format!(
                    "`{name}` is called only by code `#[endpoint]` / `routes!` generate: declare the endpoint with \
                     `#[endpoint(…)]` and list it in `routes![…]`"
                ),
            ));
        }
        // `impl Endpoint for X` (hay `impl …::http::Endpoint for X`) viết tay: chỉ `#[endpoint]` cài trait này.
        let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
        if compact.starts_with("impl") && compact.contains("Endpointfor") {
            found.push((
                index,
                "`Endpoint` is implemented only by `#[endpoint]`: declare the handler with `#[endpoint(…)]`".to_owned(),
            ));
        }
    }
    found
}

/// Thư viện mà kernel dùng ĐÚNG bản thư viện khác dùng, nên `Cargo.lock` chỉ được có một bản: `matchit` (bộ khớp route
/// của axum, kernel hỏi nó để báo route xung đột như axum sẽ thấy — D-20261127-http-layer-choices).
const SINGLE_COPY: &[&str] = &["matchit"];

/// `single-copy`: mỗi thư viện trong [`SINGLE_COPY`] có đúng một bản trong `Cargo.lock`.
pub fn check_single_copies(root: &Path) -> Result<Vec<Violation>, String> {
    let lock = fs::read_to_string(root.join("Cargo.lock"))
        .map_err(|error| format!("Cargo.lock: {error}"))?;
    Ok(single_copy_problems(&lock))
}

fn single_copy_problems(lock: &str) -> Vec<Violation> {
    SINGLE_COPY
        .iter()
        .filter_map(|name| {
            let wanted = format!("name = \"{name}\"");
            let copies = lock.lines().filter(|line| line.trim() == wanted).count();
            (copies > 1).then(|| Violation {
                rule: "single-copy",
                location: "Cargo.lock".to_owned(),
                message: format!(
                    "`{name}` appears {copies} times: the kernel must use the same version as axum; align the \
                     version in [workspace.dependencies] (D-20261127-http-layer-choices)"
                ),
            })
        })
        .collect()
}

/// File test của module: `tests.rs`, `*_tests.rs`, hay nằm trong thư mục `tests/`.
fn is_test_file(file: &str) -> bool {
    let name = file.rsplit('/').next().unwrap_or(file);
    name == "tests.rs" || name.ends_with("_tests.rs") || file.contains("/tests/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matchit_chi_mot_ban() {
        let one = "[[package]]\nname = \"matchit\"\nversion = \"0.8.4\"\n";
        assert!(single_copy_problems(one).is_empty());
        let two = format!("{one}\n[[package]]\nname = \"matchit\"\nversion = \"0.9.0\"\n");
        let found = single_copy_problems(&two);
        assert_eq!(found.len(), 1);
        assert!(found[0].message.contains("2 times"));
    }

    // Đường riêng của kernel: `__private`, hàm dựng ẩn của lớp HTTP, `impl Endpoint for` viết tay; chú thích thì bỏ qua.
    #[test]
    fn duong_rieng_cua_kernel_bi_chan() {
        let text = "use autoxide_kernel::__private::utoipa;\nlet r = routes.__add(doc, router);\n\
                    let d = EndpointDoc::__new(p, m, o, s);\nimpl autoxide_kernel::http::Endpoint for Fake {}\n\
                    impl Endpoint for Fake {}\n// __add trong chú thích\nlet added = add(1);\n";
        let lines: Vec<usize> = private_uses(text).iter().map(|(line, _)| *line).collect();
        assert_eq!(lines, [0, 1, 2, 3, 4]);
    }
    use crate::workspace::{Dep, Package};

    // Phụ thuộc thường và phụ thuộc của test tính hai luật riêng; module chỉ còn thư viện bọc ở dev vẫn là "đã chuyển".
    #[test]
    fn thu_vien_boc_tach_phu_thuoc_thuong_va_dev() {
        let root = std::env::temp_dir().join(format!("xtask-wrapped-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("xtask")).unwrap();
        fs::create_dir_all(root.join("modules/notes/src")).unwrap();
        fs::write(root.join(LIBRARIES), "# chú thích\naxum\ntower\nutoipa\n").unwrap();
        fs::write(root.join("modules/notes/src/lib.rs"), "pub fn module() {}\n").unwrap();
        let dep = |name: &str, kind| Dep { name: name.into(), kind };
        let package = Package {
            name: "autoxide-notes".into(),
            license: None,
            dir: "modules/notes".into(),
            role: Role::Module,
            deps: vec![
                dep("utoipa", DepKind::Normal),
                dep("axum", DepKind::Dev),
                dep("tower", DepKind::Dev),
                dep("serde", DepKind::Normal),
            ],
        };
        let workspace = Workspace { root: root.clone(), packages: vec![package] };
        let rules: Vec<&str> =
            check(&workspace).unwrap().iter().map(|violation| violation.rule).collect();
        assert_eq!(rules, ["wrapped-libraries", "wrapped-libraries-dev"]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn record_nhieu_dong_cfg_attr_module_long() {
        let text = "#[derive(\n    Clone,\n    Record,\n)]\npub(super) struct A {}\n#[cfg_attr(test, derive(Debug, Record))]\nstruct B {}\nmod inner { #[derive(autoxide_kernel::db::Record)] pub(in crate) struct C {} }\n#[derive(Debug)]\nstruct D;\n";
        let syntax = syn::parse_file(text).unwrap();
        let mut found = Vec::new();
        collect_records(&syntax.items, &module_path("src/report.rs"), "src/report.rs", &mut found);
        let names: Vec<String> = found.iter().map(|(path, _)| path.join("::")).collect();
        assert_eq!(names, ["report::A", "report::B", "report::inner::C"]);
    }

    #[test]
    fn so_duong_dan_dang_ky_day_du() {
        let derived = vec![
            (vec!["entity".to_owned(), "Note".to_owned()], String::new()),
            (vec!["report".to_owned(), "Note".to_owned()], String::new()),
        ];
        let registered =
            registered_paths("vec ! [RecordInfo :: of :: < crate :: entity :: Note > ()]");
        assert!(is_registered(&derived[0].0, &registered, &derived));
        assert!(
            !is_registered(&derived[1].0, &registered, &derived),
            "đuôi `Note` của record khác không tính"
        );
        let short = registered_paths("RecordInfo :: of :: < Note > ()");
        assert!(
            !is_registered(&derived[0].0, &short, &derived),
            "tên ngắn trùng hai record: không tính"
        );
    }

    #[test]
    fn tu_cam_va_feature_testing() {
        let code = "use autoxide_kernel::migration as m;\nuse m::prelude::ConnectionTrait;\n// sea_orm trong chú thích\nlet prelude_x = 1;\n";
        let words: Vec<&str> = banned_words(code).into_iter().map(|(_, word)| word).collect();
        assert_eq!(words, ["ConnectionTrait"]);
        let file = syn::parse_file(
            "use autoxide_kernel::migration as m;\nuse autoxide_kernel::migration::{\n    prelude::*,\n};\nuse proptest::prelude::*;\nuse autoxide_kernel::migration::MigrationTrait;\nmod t { use autoxide_kernel::migration::prelude::DbErr; }\n",
        )
        .unwrap();
        assert_eq!(migration_uses(&file.items).len(), 3);
        let manifest = "[dependencies.autoxide-kernel]\nworkspace = true\nfeatures = [\"testing\"]\n\n[dev-dependencies]\nautoxide-kernel = {\n  workspace = true,\n  features = [\"testing\"],\n}\n";
        assert_eq!(kernel_testing_lines(manifest), [3, 6]);
    }

    #[test]
    fn nhan_ra_file_test() {
        assert!(is_test_file("src/tests.rs"));
        assert!(is_test_file("src/sync_tests.rs"));
        assert!(is_test_file("src/http/tests/list.rs"));
        assert!(!is_test_file("src/service.rs"));
    }
}
