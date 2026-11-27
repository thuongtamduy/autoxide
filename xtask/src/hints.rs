//! Mỗi luật của `cargo xtask check` / `check-module` có một dòng "cách sửa", in ngay dưới vi phạm
//! (bước 71, G6): người mới và AI agent thấy phải làm gì mà không phải đọc mã của xtask. Tra theo tên
//! luật (phần trong `[ngoặc]` đầu dòng). Thêm luật mới thì thêm dòng ở đây — test dưới chặn quên.

/// (tên luật, cách sửa). Viết tiếng Anh: người ngoài đọc kết quả của lệnh.
const FIXES: &[(&str, &str)] = &[
    (
        "address-key",
        "Build the key with `autoxide_kernel::throttle::address_key(kind, ip)` so IPv6 clients are limited by block; if the string is already built from a bucketed value, add `// address-key: <reason>` on that line or the one above.",
    ),
    (
        "api-closed-body",
        "Add `#[serde(deny_unknown_fields)]` to the request type, or list the operation in xtask/api-shape-allow.txt only if it is an old one waiting to be fixed.",
    ),
    (
        "api-page-params",
        "Take `limit`, `cursor` and `order` through `PageQuery` (see `fetch_page`) instead of hand-written paging parameters.",
    ),
    (
        "api-param-description",
        "Put a `///` line (English) on every field of the query or path struct; it becomes the OpenAPI description.",
    ),
    (
        "api-problem",
        "Declare the error responses with the shared `Problem` body, so clients get the same error shape everywhere.",
    ),
    (
        "api-unpaged-list",
        "Return `Page<T>` (`data` + `pagination`) from list endpoints, built with `fetch_page`; never `Data<Vec<T>>`.",
    ),
    (
        "changelog-fragment",
        "Write the fragment as changelog.d/<name>.<section>.md with one non-empty bullet; do not edit CHANGELOG.md.",
    ),
    (
        "decisions-id",
        "Give the decision file a unique id `D-<YYYYMMDD>-<short-name>` and run `cargo xtask decisions`.",
    ),
    (
        "decisions-ref",
        "Point to a decision id that exists (see docs/decisions/INDEX.md after `cargo xtask decisions`), or fix the typo.",
    ),
    (
        "dependency",
        "Remove the dependency the message names: modules talk to each other only through `autoxide_<module>::api`, and the kernel depends on no module. Move the shared type into `api` or the kernel.",
    ),
    (
        "deploy-metrics",
        "Make every metric name used in deploy/prometheus and deploy/grafana one that the code records with `metrics::counter!`/`gauge!`/`histogram!`, and give every `- alert:` an `action:` line.",
    ),
    (
        "doc-language",
        "Write `///` and `//!` lines of public items in English; put Vietnamese notes in plain `//` comments.",
    ),
    (
        "docs-required",
        "Add the missing document the message names (module guides need an English and a Vietnamese file).",
    ),
    (
        "docs-translation",
        "Keep the `.md` and `.vi.md` versions in sync: change both in the same commit.",
    ),
    (
        "entity-tenant",
        "Add a `tenant_id` column to the entity (a tenant-level table); only control-plane tables, listed in the module's `control_plane_tables`, may go without.",
    ),
    (
        "env-access",
        "Do not call `std::env::var`; declare a `Setting` and read it through `ConfigReader`.",
    ),
    (
        "error-code-prefix",
        "Prefix the error code of a module with `<module>.` (for example `notes.title_taken`), in snake_case.",
    ),
    (
        "file-length",
        "Split the file by responsibility into child modules (move code only); a file already in xtask/file-length-baseline.txt must not grow.",
    ),
    (
        "i18n",
        "Add the error code to the module's `locales/vi.json` (the code is the i18n key) and list it in `Module::translations`; a translation for a code that no longer exists is deleted from the file.",
    ),
    (
        "i18n-field",
        "Give field messages a stable i18n key; do not build them with `format!` at the call site.",
    ),
    (
        "layout",
        "Move the crate to crates/kernel, crates/testkit, crates/app, modules/<name> or xtask.",
    ),
    ("license", "Add `license.workspace = true` to the crate's Cargo.toml."),
    (
        "lint-allow",
        "Remove the `#[allow(clippy::unwrap_used …)]` from src/; return an error instead. Only `#[cfg(test)]` code and tests/ may allow them.",
    ),
    ("lints", "Add `[lints]` with `workspace = true` to the crate's Cargo.toml."),
    (
        "mcp-page-count",
        "Make the MCP list tool take the same `limit`/`cursor` as the REST endpoint and return `pagination`.",
    ),
    (
        "mcp-principal",
        "Pass the `principal` into the service call from the MCP tool, so it cannot do more than REST with the same permission.",
    ),
    ("metric-labels", "Use labels from a small fixed set (an enum), never ids or raw paths."),
    (
        "permission-first",
        "Call `principal.require(&PERMISSION)?` (or the same check the service runs first) before parsing the page request, so a caller without the permission gets 403, not 422; if the order is intended, add `// permission-first: <reason>` above the `fn`.",
    ),
    (
        "private-paths",
        "Use the public path (`autoxide_kernel::db::…`, `autoxide_kernel::http::…`); `__private` is only for code the kernel's macros generate.",
    ),
    (
        "publish-ready",
        "Fill the missing crates.io field in the crate's Cargo.toml (description, license, repository, readme…), and keep the `version` pins in [workspace.dependencies] equal to the workspace version.",
    ),
    (
        "raw-database",
        "Query through `tenant.db()` (the kernel `db` layer filters by tenant); control-plane tables through `context.control_plane()`. Old modules stay in xtask/library-baseline.txt until their cluster is converted.",
    ),
    (
        "raw-sql",
        "Express the query with the `autoxide_kernel::db` builder (`find`, `select`, `sub`, `group_by`); keep `db.sql` only where it is measured to be slow, with a recorded reason and a tenant-isolation test.",
    ),
    (
        "raw-statement",
        "Build hand-written SQL with `autoxide_kernel::database::statement`, not `Statement::from_sql_and_values`.",
    ),
    (
        "record-registered",
        "Add `RecordInfo::of::<Name>()` to the module's `Module::records()`, so the startup check compares the record with its table.",
    ),
    (
        "rust-free-paths",
        "Remove the entry from scripts/rust-free-paths.txt: the file is read by Rust (a crate directory, `include_str!`, or a test that reads it at run time), so a change to it must run the Rust tests.",
    ),
    (
        "site-links",
        "Declare a `TenantSetting::link(…)` that names this `AUTOXIDE_…_URL` setting (so each tenant can set its own page), or add the variable to NOT_SITE_LINKS in xtask/src/site_links.rs if it is a partner API or platform address.",
    ),
    (
        "tenant-scope",
        "Add `.scoped(&tenant)` to the query; if scanning every tenant is on purpose, add `// tenant-scope: <reason>` above it.",
    ),
    (
        "test-binary",
        "Add the test file under tests/integration/ and a `mod <name>;` line in tests/integration/main.rs; do not create tests/<name>.rs.",
    ),
    (
        "testing-feature",
        "Remove `features = [\"testing\"]` from the dependency on `autoxide-kernel`; tests get direct table access through `autoxide_testkit::all_tenants`.",
    ),
    (
        "toolchain",
        "Change the Rust version in rust-toolchain.toml, `rust-version` in the root Cargo.toml, examples/external-module (channel and rust-version) and the Dockerfile `ARG RUST_VERSION` together.",
    ),
    (
        "wrapped-libraries",
        "Remove the dependency from the module's Cargo.toml and use the kernel layer instead (`autoxide_kernel::db` with `#[derive(Record)]` for queries; see docs/research/kernel-contract-1-0.md); a module not yet converted stays in xtask/library-baseline.txt until its cluster is converted.",
    ),
    (
        "wrapped-paths",
        "Use `autoxide_kernel::db` instead of `sea_orm`; keep `sea_orm` and `migration::prelude` only under src/migration/ (and import `MigrationTrait` from `autoxide_kernel::migration`).",
    ),
];

/// Cách sửa của một luật, nếu có.
pub fn fix(rule: &str) -> Option<&'static str> {
    FIXES.iter().find(|(name, _)| *name == rule).map(|(_, fix)| *fix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bang_xep_theo_ten_va_khong_trung() {
        let names: Vec<&str> = FIXES.iter().map(|(name, _)| *name).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(names, sorted, "keep FIXES sorted by rule name, one line per rule");
    }

    /// Mọi tên luật xuất hiện trong mã của xtask (`rule: "…"`, `const RULE: &str = "…"`, danh sách
    /// `RULES`) phải có cách sửa — thêm luật mà quên dòng gợi ý thì test này đỏ.
    #[test]
    fn moi_luat_co_cach_sua() {
        let mut missing = Vec::new();
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stack = vec![source];
        while let Some(path) = stack.pop() {
            for entry in std::fs::read_dir(&path).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    let text = std::fs::read_to_string(&path).unwrap();
                    for rule in rule_names(&text) {
                        if fix(&rule).is_none() && !missing.contains(&rule) {
                            missing.push(rule);
                        }
                    }
                }
            }
        }
        assert!(missing.is_empty(), "rules without a fix line in hints.rs: {missing:?}");
    }

    /// Tên luật viết cạnh `rule:`, `RULE: &str =` hay trong `RULES: &[&str] = &[…]` (ngoài test).
    fn rule_names(text: &str) -> Vec<String> {
        let code = text.split("#[cfg(test)]").next().unwrap_or(text);
        let mut names = Vec::new();
        for marker in ["rule: \"", "RULE: &str = \""] {
            for part in code.split(marker).skip(1) {
                if let Some(name) = part.split('"').next() {
                    names.push(name.to_owned());
                }
            }
        }
        if let Some(list) = code.split("const RULES: &[&str] = &[").nth(1) {
            let list = list.split("];").next().unwrap_or("");
            for item in list.split('"').skip(1).step_by(2) {
                names.push(item.to_owned());
            }
        }
        // Tên có dấu chấm (`response.property_removed`…) là luật của api-diff, không phải vi phạm của `check`.
        names.retain(|name| !name.is_empty() && !name.contains([' ', '.']));
        names
    }
}
