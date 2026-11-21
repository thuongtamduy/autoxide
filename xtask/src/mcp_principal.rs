//! Luật `mcp-principal` (D-20261121-security-lints): tool MCP không được rộng tay hơn REST (luật
//! kiến trúc 8). Lỗi tái phát mà đợt rà soát bảo mật tháng 11/2026 tìm thấy: REST giới hạn danh
//! sách vào phạm vi permission của người gọi (`principal.scope_condition(&P, …)` hay
//! `principal.require_at(…, &P, …)`), còn tool MCP cùng permission `P` bỏ qua `principal`
//! (`_principal: Principal` hay không dùng tới) nên thấy cả khách.
//!
//! Cách kiểm, trong mỗi crate (code đã che comment/chuỗi/test — xem `source.rs`):
//!
//! 1. Permission "theo phạm vi" = hằng `P` xuất hiện dạng `&P` trong đối số của một lời gọi
//!    `scope_condition(` hay `require_at(` ở bất kỳ đâu trong crate.
//! 2. Mỗi `Tool::new(…)` có closure nhận tham số kiểu `Principal`; tham số đó tên bắt đầu bằng `_`
//!    hay không được nhắc trong thân closure, mà `.requires(…)` của tool có permission theo phạm vi
//!    → vi phạm.
//!
//! Giới hạn: permission truyền qua biến (`scope_condition(permission, …)`) không được nhận ra;
//! handler là hàm đặt tên riêng (không phải closure) không được kiểm; dùng `principal` ở đâu đó
//! trong closure là đủ, luật không xét có truyền đúng chỗ không. Tool cố ý xem cả khách (dữ liệu
//! không gắn nút tổ chức…) ghi `// mcp-principal: <lý do>` ngay trên `Tool::new`.

use std::fs;

use crate::boundaries::{Violation, rust_files};
use crate::source::{Source, expression_end, find_word, is_ident_byte, is_test_file};
use crate::workspace::{Role, Workspace};

pub const RULE: &str = "mcp-principal";
/// Chú thích miễn trừ cho một tool.
pub const MARKER: &str = "mcp-principal:";
const SCOPE_CALLS: &[&str] = &["scope_condition(", "require_at("];

pub fn check(workspace: &Workspace) -> Result<Vec<Violation>, String> {
    let mut violations = Vec::new();
    for package in &workspace.packages {
        if !matches!(package.role, Role::Module | Role::Kernel) {
            continue;
        }
        let dir = workspace.root.join(&package.dir);
        let mut files = Vec::new();
        for file in rust_files(&dir.join("src"))? {
            // `file` có dạng `src/a/b.rs`.
            let path = format!("{}/{file}", package.dir);
            if is_test_file(&path) {
                continue;
            }
            let text = fs::read_to_string(dir.join(&file))
                .map_err(|error| format!("cannot read {path}: {error}"))?;
            files.push((path, Source::new(&text)));
        }
        violations.extend(check_crate(&files));
    }
    Ok(violations)
}

/// Vi phạm của một crate: (đường dẫn, mã nguồn) của mọi file `src/` không phải test.
pub fn check_crate(files: &[(String, Source)]) -> Vec<Violation> {
    let scoped: Vec<String> =
        files.iter().flat_map(|(_, source)| scoped_permissions(&source.code)).collect();
    let mut violations = Vec::new();
    for (path, source) in files {
        for tool in tools(source) {
            if !tool.ignores_principal {
                continue;
            }
            let Some(permission) = tool.requires.iter().find(|p| scoped.contains(p)) else {
                continue;
            };
            let line = source.line_of(tool.start);
            if source.has_marker(line, MARKER) || source.has_marker(tool.principal_line, MARKER) {
                continue;
            }
            violations.push(Violation {
                rule: RULE,
                location: format!("{path}:{line}"),
                message: format!(
                    "MCP tool `{}` requires `{permission}`, which this crate limits to the \
                     caller's org scope (`scope_condition` / `require_at`), but the tool ignores \
                     `principal`: pass `&principal` to the service method that applies the scope, \
                     as the REST handler does; if the tool really sees the whole tenant, write \
                     `// {MARKER} <why>` above `Tool::new`",
                    tool.name
                ),
            });
        }
    }
    violations
}

/// Hằng permission `P` có dạng `&P` trong đối số của `scope_condition(` / `require_at(`. Đối số
/// đầu của `scope_condition` là permission, của `require_at` là kết nối rồi permission — xét mọi
/// đối số cho đơn giản.
fn scoped_permissions(code: &str) -> Vec<String> {
    let mut found = Vec::new();
    for call in SCOPE_CALLS {
        let mut search = 0;
        while let Some(offset) = code[search..].find(call) {
            let mut at = search + offset + call.len();
            search = at;
            // `expression_end` dừng ở `,` cùng cấp: mỗi vòng là một đối số.
            loop {
                let end = expression_end(code, at);
                found.extend(constant(code[at..end].trim()));
                if code.as_bytes().get(end) != Some(&b',') {
                    break;
                }
                at = end + 1;
            }
        }
    }
    found
}

/// `&P`, `&crate::P` (P viết HOA) → `P`.
fn constant(argument: &str) -> Option<String> {
    let path = argument.strip_prefix('&')?.trim();
    let name = path.rsplit("::").next()?;
    let valid = name.starts_with(|c: char| c.is_ascii_uppercase())
        && name.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_');
    valid.then(|| name.to_owned())
}

struct Tool {
    start: usize,
    name: String,
    requires: Vec<String>,
    ignores_principal: bool,
    principal_line: usize,
}

fn tools(source: &Source) -> Vec<Tool> {
    let code = &source.code;
    let mut found = Vec::new();
    for at in find_word(code, "Tool") {
        let rest = &code[at + 4..];
        if !rest.starts_with("::new(") {
            continue;
        }
        let open = at + 4 + "::new(".len();
        let close = call_end(code, open);
        let body = &code[open..close];
        // Tên tool: chuỗi đầu tiên trong đối số (bản nguyên văn, vì `code` đã che chuỗi).
        let raw_body = &source.raw[open..close];
        let name = raw_body.split('"').nth(1).unwrap_or("?").to_owned();
        let Some((principal_at, ignored)) = principal_param(body) else { continue };
        let after = (close + 1).min(code.len());
        let tail = &code[after..expression_end(code, after)];
        let requires = requires(tail);
        found.push(Tool {
            start: at,
            name,
            requires,
            ignores_principal: ignored,
            principal_line: source.line_of(open + principal_at),
        });
    }
    found
}

/// Vị trí `)` đóng lời gọi có `(` ngay trước `open`.
fn call_end(code: &str, open: usize) -> usize {
    let mut depth = 0i32;
    for (offset, byte) in code.as_bytes()[open..].iter().enumerate() {
        match byte {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth < 0 {
                    return open + offset;
                }
            }
            _ => {}
        }
    }
    code.len()
}

/// Tham số kiểu `Principal` của closure đầu tiên trong `body`: (vị trí, có bị bỏ qua không).
fn principal_param(body: &str) -> Option<(usize, bool)> {
    let first = body.find('|')?;
    let second = first + 1 + body[first + 1..].find('|')?;
    let mut offset = first + 1;
    for param in body[first + 1..second].split(',') {
        let here = offset;
        offset += param.len() + 1;
        let Some((name, kind)) = param.split_once(':') else { continue };
        if kind.trim() != "Principal" && !kind.trim().ends_with("::Principal") {
            continue;
        }
        let name = name.trim().trim_start_matches("mut ").trim().to_owned();
        let closure = &body[second + 1..];
        let ignored = name.starts_with('_') || find_word(closure, &name).is_empty();
        let position = here + param.len() - param.trim_start().len();
        return Some((position, ignored));
    }
    None
}

/// Hằng trong `.requires(…)` của chuỗi gọi sau `Tool::new(…)`.
fn requires(tail: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut search = 0;
    while let Some(offset) = tail[search..].find(".requires(") {
        let open = search + offset + ".requires(".len();
        search = open;
        let close = call_end(tail, open);
        for word in tail[open..close].split(|c: char| !(is_ident_byte(c as u8) || c == ':')) {
            let name = word.rsplit("::").next().unwrap_or("");
            if constant(&format!("&{name}")).is_some() {
                found.push(name.to_owned());
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERVICE: &str = "impl Service {
    fn list(&self, principal: &Principal) {
        let scope = principal.scope_condition(&PRODUCTS_READ, Column::OrgId, None)?;
        principal
            .require_at(
                db,
                &crate::PRODUCTS_WRITE,
                target,
            )
            .await?;
    }
}
";

    fn tool(closure: &str, permission: &str) -> String {
        format!(
            "fn tools() -> Vec<Tool> {{
    vec![
        Tool::new(
            \"list_products\",
            \"List products.\",
            {closure},
        )
        .requires({permission})
        .read_only(),
    ]
}}
"
        )
    }

    fn check(tools_rs: &str) -> Vec<Violation> {
        let files = [
            ("shop/src/service.rs".to_owned(), Source::new(SERVICE)),
            ("shop/src/tools.rs".to_owned(), Source::new(tools_rs)),
        ];
        check_crate(&files)
    }

    #[test]
    fn tool_dung_principal_khong_bi_bao() {
        let used = "move |tenant: TenantContext, principal: Principal, input: In| {
                let service = service.clone();
                async move { service.list(&tenant, &principal).await }
            }";
        assert!(check(&tool(used, "PRODUCTS_READ")).is_empty());
        // Permission không theo phạm vi ở đâu trong crate: bỏ qua principal không sao.
        let ignored = "move |tenant: TenantContext, _principal: Principal, _input: EmptyInput| {
                async move { service.count(&tenant).await }
            }";
        assert!(check(&tool(ignored, "SETTINGS_READ")).is_empty());
    }

    #[test]
    fn tool_bo_qua_principal_voi_permission_theo_pham_vi_bi_bao() {
        let ignored = "move |tenant: TenantContext, _principal: Principal, _input: EmptyInput| {
                async move { service.list_all(&tenant).await }
            }";
        let found = check(&tool(ignored, "PRODUCTS_READ"));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].location, "shop/src/tools.rs:3");
        assert!(found[0].message.contains("`list_products`"), "{}", found[0].message);
        assert!(found[0].message.contains("pass `&principal`"), "{}", found[0].message);
        assert!(found[0].message.contains("// mcp-principal: <why>"), "{}", found[0].message);
        // Đặt tên không gạch dưới mà không dùng cũng bị bắt; permission ghi kèm đường dẫn.
        let unused = "move |tenant: TenantContext, principal: Principal, _input: EmptyInput| {
                async move { service.list_all(&tenant).await }
            }";
        assert_eq!(check(&tool(unused, "crate::PRODUCTS_WRITE")).len(), 1);
    }

    #[test]
    fn chu_thich_mien_tru() {
        let ignored = "move |tenant: TenantContext, _principal: Principal, _input: EmptyInput| {
                async move { service.list_all(&tenant).await }
            }";
        let text = tool(ignored, "PRODUCTS_READ").replace(
            "        Tool::new(",
            "        // mcp-principal: danh mục chung của khách, không gắn nút tổ chức.\n        Tool::new(",
        );
        assert!(check(&text).is_empty());
    }

    #[test]
    fn doc_permission_theo_pham_vi() {
        let mut found = scoped_permissions(SERVICE);
        found.sort();
        assert_eq!(found, ["PRODUCTS_READ", "PRODUCTS_WRITE"]);
    }
}
