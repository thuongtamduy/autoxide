//! Luật `permission-first` (D-20261126-permission-before-page): handler danh sách kiểm quyền TRƯỚC khi đọc tham số
//! trang. Người gọi thiếu quyền gửi `limit=0` phải nhận 403 `permission_denied`, không phải 422 — 422 cho biết
//! endpoint có, tham số nào sai, trước khi biết người gọi có được hỏi không.
//!
//! Lớp kiểm `x-permissions` của kernel (D-20261126-permission-gate) đã chặn người không có permission nào trong
//! danh sách, nhưng nó chỉ hỏi "có ở một đơn vị nào đó" và không hỏi xác nhận lại (step-up): người qua được lớp
//! đó mà service vẫn từ chối (permission cần step-up, danh sách "một trong" mà service cần một cái cụ thể, route
//! dựng không kèm lớp đó) sẽ thấy 422 trước 403 nếu handler đọc trang trước khi service kiểm quyền.
//!
//! Cách kiểm, trên mỗi hàm có `#[utoipa::path(…)]` (code đã che comment/chuỗi/test — xem `source.rs`):
//!
//! 1. Chỗ đọc trang đầu tiên trong thân hàm: `PageRequest::try_from(` hay `.into_request(`. Không có thì bỏ qua.
//! 2. "Đã kiểm quyền" = trước chỗ đó có một câu lệnh trọn (kết thúc bằng `;`) nhắc tới `principal` và có `?` hay
//!    `return Err` — `principal.require(&P)?`, `require_cod_read(&principal)?`, `check_admin(…, &principal, …)
//!    .await?`… (`Actor::from(&principal)` không có `?` nên không tính).
//! 3. Vi phạm khi chưa kiểm quyền mà: operation khai `x-permissions` khác rỗng (trừ `x-buyers: true`, nơi khách
//!    mua không có permission và service tự quyết); hoặc sau chỗ đọc trang thân hàm mới gọi
//!    `principal.require…(…)?` / `require_platform(&principal)?`.
//!
//! Giới hạn: đọc trang trong hàm phụ (không phải hai mẫu trên) không được nhận ra; handler `x-permissions: []` để
//! service kiểm quyền sau khi đã đọc trang cũng không. Cố ý đọc trang trước thì ghi
//! `// permission-first: <lý do>` ngay trên `fn`.

use std::fs;
use std::path::Path;

use crate::boundaries::Violation;
use crate::source::{Source, find_word, is_test_file};

pub const RULE: &str = "permission-first";
/// Chú thích miễn trừ cho một handler.
pub const MARKER: &str = "permission-first:";
const PARSES: &[&str] = &["PageRequest::try_from(", ".into_request("];

pub fn check(root: &Path) -> Result<Vec<Violation>, String> {
    let mut violations = Vec::new();
    for file in crate::lengths::rust_files(root)? {
        if file.starts_with("xtask/") || !file.contains("/src/") || is_test_file(&file) {
            continue;
        }
        let text = fs::read_to_string(root.join(&file))
            .map_err(|error| format!("cannot read {file}: {error}"))?;
        if !text.contains("utoipa::path") {
            continue;
        }
        violations.extend(check_text(&file, &text));
    }
    Ok(violations)
}

/// Phần không đọc đĩa của [`check`], để test được.
fn check_text(file: &str, text: &str) -> Vec<Violation> {
    let source = Source::new(text);
    let code = &source.code;
    let mut violations = Vec::new();
    let mut search = 0;
    while let Some(offset) = code[search..].find("#[utoipa::path(") {
        let open = search + offset + "#[utoipa::path".len();
        search = open;
        let Some(close) = matching(code, open, b'(', b')') else { break };
        // Chuỗi trong `code` đã bị che: đọc khai báo ở bản nguyên văn, cùng vị trí.
        let attribute = &source.raw[open..close];
        let guarded = declares_permissions(attribute) && !attribute.contains("\"x-buyers\"");
        let Some(function) = find_word(&code[close..], "fn").first().map(|at| close + at) else {
            break;
        };
        let Some(body) = body_of(code, function) else { continue };
        let body_text = &code[body.0..body.1];
        let Some(parse) = PARSES.iter().filter_map(|pattern| body_text.find(pattern)).min() else {
            continue;
        };
        if checked_before(&body_text[..parse]) {
            continue;
        }
        let late = refusing_check(&body_text[parse..]);
        if !guarded && !late {
            continue;
        }
        let line = source.line_of(function);
        if source.has_marker(line, MARKER) {
            continue;
        }
        let name: String = code[function + 2..]
            .trim_start()
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        violations.push(Violation {
            rule: RULE,
            location: format!("{file}:{line}"),
            message: format!(
                "handler `{name}` reads the page parameters before checking the caller's permission, so a \
                 caller without it who sends a bad `limit` gets 422 instead of 403: call \
                 `principal.require(&P)?` (or the check the service does first) before \
                 `PageRequest::try_from` / `into_request`; if the order is intended, write \
                 `// {MARKER} <why>` above `fn`"
            ),
        });
    }
    violations
}

/// `extensions(("x-permissions" = json!([...])))` với ít nhất một tên.
fn declares_permissions(attribute: &str) -> bool {
    let Some(at) = attribute.find("\"x-permissions\"") else { return false };
    let rest = &attribute[at..];
    let Some(start) = rest.find('[') else { return false };
    let Some(end) = rest[start..].find(']') else { return false };
    rest[start..start + end].contains('"')
}

/// Câu lệnh trọn trước chỗ đọc trang có nhắc `principal` và có thể dừng hàm bằng lỗi.
fn checked_before(prefix: &str) -> bool {
    let Some(last) = prefix.rfind(';') else { return false };
    prefix[..last].split(';').any(|statement| {
        !find_word(statement, "principal").is_empty()
            && (statement.contains('?') || statement.contains("return Err"))
    })
}

/// Sau chỗ đọc trang có lời gọi kiểm quyền trả lỗi: `principal.require…(…)?` hay `require_platform(&principal)?`.
/// `principal.require(&P).is_ok()` (chỉ để lọc) không tính.
fn refusing_check(rest: &str) -> bool {
    let bytes = rest.as_bytes();
    let followed_by_question = |open: usize| {
        matching(rest, open, b'(', b')')
            .is_some_and(|close| rest[close + 1..].trim_start().starts_with('?'))
    };
    let mut search = 0;
    while let Some(offset) = rest[search..].find("principal.require") {
        let at = search + offset + "principal.require".len();
        search = at;
        let mut open = at;
        while open < bytes.len() && (bytes[open].is_ascii_alphanumeric() || bytes[open] == b'_') {
            open += 1;
        }
        if bytes.get(open) == Some(&b'(') && followed_by_question(open) {
            return true;
        }
    }
    find_word(rest, "require_platform")
        .into_iter()
        .any(|at| followed_by_question(at + "require_platform".len()))
}

/// Thân `{ … }` của hàm bắt đầu ở `function` (vị trí chữ `fn`): sau ngoặc đóng của danh sách tham số.
fn body_of(code: &str, function: usize) -> Option<(usize, usize)> {
    let open = function + code[function..].find('(')?;
    let close = matching(code, open, b'(', b')')?;
    let start = close + code[close..].find('{')?;
    let end = matching(code, start, b'{', b'}')?;
    Some((start, end))
}

/// Vị trí ngoặc đóng khớp với ngoặc mở ở `open`.
fn matching(code: &str, open: usize, left: u8, right: u8) -> Option<usize> {
    let mut depth = 0i32;
    for (offset, byte) in code.as_bytes().get(open..)?.iter().enumerate() {
        if *byte == left {
            depth += 1;
        } else if *byte == right {
            depth -= 1;
            if depth == 0 {
                return Some(open + offset);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handler(permissions: &str, body: &str) -> String {
        format!(
            "#[utoipa::path(\n    get,\n    path = \"/\",\n    extensions((\"x-permissions\" = json!([{permissions}]))),\n)]\n\
             async fn list_things(\n    principal: Principal,\n    Query(page): Query<PageQuery>,\n\
             ) -> Result<Page<Thing>, Error> {{\n{body}\n}}\n"
        )
    }

    #[test]
    fn doc_trang_truoc_khi_kiem_quyen_bi_chan() {
        let late = handler(
            "\"things.read\"",
            "    let page = PageRequest::try_from(page)?;\n    service.list(&principal, &page).await",
        );
        let found = check_text("modules/x/src/http.rs", &late);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].location, "modules/x/src/http.rs:6");
        assert!(found[0].message.contains("list_things"));

        // Trong cùng biểu thức đối số: vẫn đọc trang trước khi service kiểm.
        let inline = handler(
            "\"things.read\"",
            "    service.list(&principal, &PageRequest::try_from(page)?).await",
        );
        assert_eq!(check_text("a.rs", &inline).len(), 1);

        // `x-permissions: []` nhưng thân hàm tự kiểm (trả lỗi) sau khi đọc trang.
        let empty = handler(
            "",
            "    let page = query.into_request(Order::Asc)?;\n    principal.require(&THINGS_READ)?;\n    Ok(page)",
        );
        assert_eq!(check_text("a.rs", &empty).len(), 1);
    }

    #[test]
    fn kiem_quyen_truoc_hay_mien_tru_thi_qua() {
        let first = handler(
            "\"things.read\"",
            "    principal.require(&THINGS_READ)?;\n    let page = PageRequest::try_from(page)?;\n    Ok(page)",
        );
        assert!(check_text("a.rs", &first).is_empty());
        let helper = handler(
            "\"things.read\"",
            "    check_admin(&state, &principal, id).await?;\n    let page = PageRequest::try_from(page)?;",
        );
        assert!(check_text("a.rs", &helper).is_empty());
        let branch = handler(
            "\"a\", \"b\"",
            "    if !principal.has(&A) {\n        return Err(denied());\n    }\n    let page = PageRequest::try_from(page)?;",
        );
        assert!(check_text("a.rs", &branch).is_empty());

        // `Actor::from(&principal)` không phải kiểm quyền.
        let actor = handler(
            "\"things.read\"",
            "    let actor = Actor::from(&principal);\n    let page = PageRequest::try_from(page)?;",
        );
        assert_eq!(check_text("a.rs", &actor).len(), 1);

        // Không khai permission, chỉ dùng `require` để lọc: qua.
        let filter = handler(
            "",
            "    let page = PageRequest::try_from(page)?;\n    let all = principal.require(&THINGS_READ).is_ok();",
        );
        assert!(check_text("a.rs", &filter).is_empty());

        // Khách mua đi qua lớp kiểm mà không có permission: service tự quyết.
        let buyers = handler(
            "\"things.use\"",
            "    let page = PageRequest::try_from(page)?;\n    service.wallet(principal.user_id(), &page).await",
        )
        .replace("json!([\"things.use\"])))", "json!([\"things.use\"])), (\"x-buyers\" = json!(true)))");
        assert!(check_text("a.rs", &buyers).is_empty());

        let marked = handler(
            "\"things.read\"",
            "    let page = PageRequest::try_from(page)?;\n    service.list(&principal, &page).await",
        )
        .replace("async fn", "// permission-first: thử nghiệm cố ý\nasync fn");
        assert!(check_text("a.rs", &marked).is_empty());
    }
}
