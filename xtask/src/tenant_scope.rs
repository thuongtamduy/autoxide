//! Luật `tenant-scope` (D-20261121-security-lints): truy vấn trên entity cấp khách hàng phải lọc
//! theo khách bằng `.scoped(&tenant)` (`autoxide_kernel::tenancy::ScopeExt`, D-253). Quên lọc là
//! lộ dữ liệu giữa các khách — lỗi đợt rà soát bảo mật tháng 11/2026 tìm thấy nhiều lần.
//!
//! Cách kiểm (theo văn bản, trên code đã che comment/chuỗi/test — xem `source.rs`):
//!
//! 1. Entity cấp khách = entity mà crate có `impl TenantScoped for …` (chỉ entity đó mới gọi được
//!    `.scoped`). Entity được gọi theo tên module chứa nó (`note::Entity` → `note`; `Entity` trong
//!    `mod note { … }` hay file `note.rs` → `note`). Tên mà crate có nhiều entity, chỉ một phần cài
//!    `TenantScoped`, là mơ hồ → bỏ qua (thà sót còn hơn báo nhầm).
//! 2. Mỗi lời gọi `<đường dẫn>::{find, find_by_id, update_many, delete_many, delete_by_id}(` trên
//!    entity đó phải có `.scoped(` trong cùng biểu thức; hoặc biểu thức được gán `let q = …;` và
//!    sau đó trong cùng khối có `q.scoped(`. `Entity` trần hay tên khác (`Notes::find`) được tra
//!    qua `use …::note::{Entity}` / `use …::note::Entity as Notes` của file.
//!
//! Giới hạn (cố ý, để ít báo nhầm): không theo dõi truy vấn truyền qua hàm khác, không thấy entity
//! gọi qua tham số generic (`E::find()`), không kiểm `Entity::update(model)` / `Entity::delete`
//! (một dòng, đã `.validate()?.scoped(…)` theo D-253). Chỗ cố ý không lọc (đọc mọi khách trong job
//! nền của mặt điều khiển…) ghi `// tenant-scope: <lý do>` ngay trên câu lệnh.

use std::collections::BTreeMap;
use std::fs;

use crate::boundaries::{Violation, rust_files};
use crate::source::{
    Source, enclosing_block_end, expression_end, find_word, is_ident_byte, is_test_file,
};
use crate::workspace::{Role, Workspace};

pub const RULE: &str = "tenant-scope";
/// Chú thích miễn trừ cho một câu lệnh.
pub const MARKER: &str = "tenant-scope:";
const METHODS: &[&str] = &["find", "find_by_id", "update_many", "delete_many", "delete_by_id"];

/// Một file đã đọc của crate: đường dẫn từ gốc repo, tên module của file, mã nguồn.
pub struct File {
    pub path: String,
    pub module: String,
    pub source: Source,
}

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
            let relative = file.strip_prefix("src/").unwrap_or(&file);
            files.push(File { module: file_module(relative), path, source: Source::new(&text) });
        }
        violations.extend(check_crate(&files));
    }
    Ok(violations)
}

/// Tên module của file theo đường dẫn trong `src/`: `a/b.rs` → `b`, `a/mod.rs` → `a`.
pub fn file_module(relative: &str) -> String {
    let path = relative.trim_end_matches(".rs");
    let mut parts: Vec<&str> = path.split('/').collect();
    if parts.last() == Some(&"mod") {
        parts.pop();
    }
    match parts.last() {
        Some(&("lib" | "main")) | None => String::new(),
        Some(name) => (*name).to_owned(),
    }
}

/// Vi phạm của một crate (mọi file `src/` không phải test của nó).
pub fn check_crate(files: &[File]) -> Vec<Violation> {
    let scoped = scoped_entities(files);
    let mut violations = Vec::new();
    for file in files {
        for query in queries(file) {
            let Some(entity) = resolve(file, &query) else { continue };
            if !scoped.contains(&entity) || is_scoped(file, &query) {
                continue;
            }
            let line = file.source.line_of(query.start);
            if file.source.has_marker(line, MARKER) {
                continue;
            }
            violations.push(Violation {
                rule: RULE,
                location: format!("{}:{line}", file.path),
                message: format!(
                    "`{}::{}(…)` on tenant-level entity `{entity}` is not filtered by tenant: add \
                     `.scoped(&tenant)` to the query (autoxide_kernel::tenancy::ScopeExt), or, if \
                     it deliberately spans tenants, write `// {MARKER} <why>` above the statement",
                    query.path, query.method
                ),
            });
        }
    }
    violations
}

/// Tên entity cấp khách hàng không mơ hồ của crate.
fn scoped_entities(files: &[File]) -> Vec<String> {
    let mut entities: BTreeMap<String, usize> = BTreeMap::new();
    let mut scoped: BTreeMap<String, usize> = BTreeMap::new();
    for file in files {
        let code = &file.source.code;
        let mods = inline_mods(code);
        for at in find_word(code, "table_name") {
            *entities.entry(module_at(file, &mods, at)).or_default() += 1;
        }
        let mut search = 0;
        while let Some(offset) = code[search..].find("TenantScoped for ") {
            let at = search + offset;
            search = at + "TenantScoped for ".len();
            let target = path_after(code, search);
            let segments: Vec<&str> = target.split("::").collect();
            let name = match segments.as_slice() {
                ["Entity"] => module_at(file, &mods, at),
                [.., parent, "Entity"] => (*parent).to_owned(),
                _ => continue,
            };
            *scoped.entry(name).or_default() += 1;
        }
        // Crate khai `impl TenantScoped for Entity` qua macro riêng (`tenant_scoped!();` trong
        // `mod <entity> { … }`): chữ `TenantScoped for` chỉ có một lần trong `macro_rules!`, nên
        // trước đây cả crate lọt khỏi luật (rà soát bảo mật 2026-11). Mỗi lần gọi macro tính như
        // một `impl` cho entity của `mod` bao nó.
        for at in find_word(code, "tenant_scoped") {
            if code[at + "tenant_scoped".len()..].trim_start().starts_with('!')
                && !code[..at].trim_end().ends_with("macro_rules!")
            {
                *scoped.entry(module_at(file, &mods, at)).or_default() += 1;
            }
        }
    }
    scoped
        .into_iter()
        .filter(|(name, count)| entities.get(name) == Some(count))
        .map(|(name, _)| name)
        .collect()
}

/// `mod tên { … }` trong file: (vị trí `{`, vị trí `}` khớp, tên).
fn inline_mods(code: &str) -> Vec<(usize, usize, String)> {
    let mut mods = Vec::new();
    for at in find_word(code, "mod") {
        let rest = &code[at + 3..];
        let name: String =
            rest.trim_start().bytes().take_while(|b| is_ident_byte(*b)).map(char::from).collect();
        let after = rest.trim_start()[name.len()..].trim_start();
        if name.is_empty() || !after.starts_with('{') {
            continue;
        }
        let open = code.len() - after.len();
        mods.push((open, enclosing_block_end(code, open + 1), name));
    }
    mods
}

/// Module chứa vị trí `at`: `mod` trong file sâu nhất bao nó, không có thì module của file.
fn module_at(file: &File, mods: &[(usize, usize, String)], at: usize) -> String {
    mods.iter()
        .filter(|(open, close, _)| *open < at && at < *close)
        .max_by_key(|(open, _, _)| *open)
        .map_or_else(|| file.module.clone(), |(_, _, name)| name.clone())
}

/// Đường dẫn `a::b::C` bắt đầu ở `from` (bỏ khoảng trắng đầu).
fn path_after(code: &str, from: usize) -> String {
    code[from..]
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == ':')
        .collect()
}

/// Một lời gọi `path::method(` tìm thấy.
pub struct Query {
    pub start: usize,
    pub path: String,
    pub method: &'static str,
}

fn queries(file: &File) -> Vec<Query> {
    let code = &file.source.code;
    let bytes = code.as_bytes();
    let mut found = Vec::new();
    for method in METHODS {
        let needle = format!("::{method}");
        let mut search = 0;
        while let Some(offset) = code[search..].find(&needle) {
            let at = search + offset;
            search = at + needle.len();
            let after = &code[search..];
            if after.bytes().next().is_some_and(is_ident_byte)
                || !after.trim_start().starts_with('(')
            {
                continue;
            }
            let mut start = at;
            while start > 0 && (is_ident_byte(bytes[start - 1]) || bytes[start - 1] == b':') {
                start -= 1;
            }
            // `.x::find(` là gọi method trên một giá trị, không phải entity.
            if start == at || (start > 0 && bytes[start - 1] == b'.') {
                continue;
            }
            found.push(Query { start, path: code[start..at].to_owned(), method });
        }
    }
    found.sort_by_key(|query| query.start);
    found
}

/// Tên module của entity mà truy vấn nhắm tới, nếu đoán chắc được.
fn resolve(file: &File, query: &Query) -> Option<String> {
    let segments: Vec<&str> = query.path.split("::").collect();
    match segments.as_slice() {
        [.., parent, "Entity"] => Some((*parent).to_owned()),
        ["Entity"] => {
            let mods = inline_mods(&file.source.code);
            let here = module_at(file, &mods, query.start);
            let defines = find_word(&file.source.code, "table_name")
                .into_iter()
                .any(|at| module_at(file, &mods, at) == here);
            if defines {
                return Some(here);
            }
            imported(&file.source.code, "Entity", None)
        }
        [alias] => imported(&file.source.code, "Entity", Some(alias)),
        _ => None,
    }
}

/// Module cha của `Entity` mà file `use` (hay `Entity as alias`, `type alias = x::Entity`). Nhiều
/// module khác nhau → không đoán.
fn imported(code: &str, item: &str, alias: Option<&str>) -> Option<String> {
    let mut parents: Vec<String> = Vec::new();
    for at in find_word(code, "use").into_iter().chain(find_word(code, "type")) {
        let end = code[at..].find(';').map_or(code.len(), |p| at + p);
        let statement: String = code[at..end].split_whitespace().collect::<Vec<_>>().join(" ");
        let statement = statement.replace(" ::", "::").replace(":: ", "::");
        for position in find_word(&statement, item) {
            let tail = statement[position + item.len()..].trim_start();
            let named = match (alias, tail.strip_prefix("as ")) {
                (None, None) => true,
                (Some(alias), Some(rest)) => {
                    rest.split(|c: char| !c.is_ascii_alphanumeric() && c != '_').next()
                        == Some(alias)
                }
                (Some(alias), None) => {
                    statement.starts_with("type ")
                        && statement[4..].trim_start().starts_with(alias)
                        && statement.contains('=')
                }
                (None, Some(_)) => false,
            };
            if named
                && let Some(parent) = parent_of(&statement, position)
                && !parents.contains(&parent)
            {
                parents.push(parent);
            }
        }
    }
    (parents.len() == 1).then(|| parents.remove(0))
}

/// Module cha của mục ở `position` trong một câu `use`: `a::b::Entity` → `b`;
/// `a::b::{Column, Entity}` → `b`.
fn parent_of(statement: &str, position: usize) -> Option<String> {
    let before = &statement[..position];
    let segment = if let Some(path) = before.strip_suffix("::") {
        path
    } else {
        // Lùi tới `{` chưa đóng chứa mục này.
        let mut depth = 0i32;
        let mut open = None;
        for (index, byte) in before.bytes().enumerate().rev() {
            match byte {
                b'}' => depth += 1,
                b'{' if depth == 0 => {
                    open = Some(index);
                    break;
                }
                b'{' => depth -= 1,
                _ => {}
            }
        }
        before[..open?].strip_suffix("::")?
    };
    let name: String = segment
        .bytes()
        .rev()
        .take_while(|b| is_ident_byte(*b))
        .map(char::from)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    (!name.is_empty() && name != "self" && name != "super" && name != "crate").then_some(name)
}

/// Truy vấn đã lọc theo khách: biểu thức có `.scoped(` hay tự lọc cột `TenantId`; khoá chính ghép
/// bắt đầu bằng khách (`find_by_id((tenant…, id))`); hoặc biểu thức nằm trong `let q = …;` rồi sau
/// đó trong cùng khối có `q.scoped(`.
fn is_scoped(file: &File, query: &Query) -> bool {
    let code = &file.source.code;
    let end = expression_end(code, query.start);
    let expression = &code[query.start..end];
    if expression.contains(".scoped(") || expression.contains("TenantId") {
        return true;
    }
    if query.method.ends_with("_by_id") {
        // `find_by_id((tenant.id().as_uuid(), id))` hay `find_by_id(tenant)`: khoá có khách.
        let after = query.start + query.path.len() + 2 + query.method.len();
        let key = code[after..].trim_start_matches([' ', '\n', '(']);
        let first = key.split([',', ')']).next().unwrap_or("");
        if first.contains("tenant") {
            return true;
        }
    }
    let Some((binding, statement_end)) = enclosing_let(code, query.start) else { return false };
    let block = &code[statement_end..enclosing_block_end(code, statement_end)];
    // `q.scoped(…)`, `q.filter(…).scoped(…)`: chuỗi gọi bắt đầu từ `q` có `.scoped(`.
    find_word(block, &binding).into_iter().any(|at| {
        let chain = &block[at..expression_end(block, at)];
        chain[binding.len()..].trim_start().starts_with('.') && chain.contains(".scoped(")
    })
}

/// Câu `let [mut] q[: T] = …;` gần nhất chứa vị trí `at` → (`q`, vị trí `;` của câu).
fn enclosing_let(code: &str, at: usize) -> Option<(String, usize)> {
    for start in find_word(&code[..at], "let").into_iter().rev().take(20) {
        let rest = code[start + 3..].trim_start();
        let rest = rest.strip_prefix("mut ").unwrap_or(rest).trim_start();
        let name: String = rest.bytes().take_while(|b| is_ident_byte(*b)).map(char::from).collect();
        let pattern_end = rest[name.len()..].trim_start();
        let simple = !name.is_empty()
            && name.starts_with(|c: char| c.is_ascii_lowercase() || c == '_')
            && (pattern_end.starts_with('=') || pattern_end.starts_with(':'));
        let end = statement_end(code, start);
        if end <= at {
            // Câu này đã kết thúc trước truy vấn; câu bao ngoài (nếu có) ở xa hơn.
            continue;
        }
        return simple.then_some((name, end));
    }
    None
}

/// Vị trí `;` kết thúc câu lệnh bắt đầu ở `start` (hay ngoặc đóng của khối bao ngoài).
fn statement_end(code: &str, start: usize) -> usize {
    let mut depth = 0i32;
    for (offset, byte) in code.as_bytes()[start..].iter().enumerate() {
        match byte {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth < 0 {
                    return start + offset;
                }
            }
            b';' if depth == 0 => return start + offset,
            _ => {}
        }
    }
    code.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, text: &str) -> File {
        let relative = path.split("/src/").nth(1).unwrap_or(path);
        File { path: path.to_owned(), module: file_module(relative), source: Source::new(text) }
    }

    const ENTITY: &str = "pub mod note {
    #[sea_orm(table_name = \"notes_note\")]
    pub struct Model {
        pub tenant_id: Uuid,
    }
    impl TenantScoped for Entity {
        fn tenant_column() -> Self::Column {
            Column::TenantId
        }
    }
}
pub mod tenant {
    #[sea_orm(table_name = \"kernel_tenants\")]
    pub struct Model {}
}
";

    fn check(service: &str) -> Vec<Violation> {
        let files = [file("notes/src/entity.rs", ENTITY), file("notes/src/service.rs", service)];
        check_crate(&files)
    }

    #[test]
    fn truy_van_da_loc_theo_khach_khong_bi_bao() {
        let service = "use crate::entity::{note, tenant};
fn a(t: &TenantContext) {
    let one = note::Entity::find_by_id(id).scoped(t).one(db).await?;
    note::Entity::delete_many()
        .filter(note::Column::OwnerId.eq(owner))
        .scoped(t)
        .exec(db)
        .await?;
    let mut select = note::Entity::find().filter(x);
    if y {
        select = select.filter(z);
    }
    let rows = select.scoped(t).all(db).await?;
    let select = if y {
        note::Entity::find().filter(a)
    } else {
        note::Entity::find().filter(b)
    };
    let page = filter.apply(select.scoped(t))?;
    let own = note::Entity::find_by_id((tenant.id().as_uuid(), id)).one(db).await?;
    note::Entity::delete_by_id((line.tenant_id, line.id)).exec(db).await?;
    let mut update = note::Entity::update_many().col_expr(a, b);
    update = update.col_expr(c, d);
    update.filter(e).scoped(t).exec(db).await?;
    let manual = note::Entity::find().filter(note::Column::TenantId.eq(t.id())).one(db).await?;
    // Bảng mặt điều khiển, không có khách.
    let all = tenant::Entity::find().all(db).await?;
}
";
        assert!(check(service).is_empty(), "{:?}", check(service));
    }

    #[test]
    fn truy_van_quen_loc_bi_bao_kem_cach_sua() {
        let service = "use crate::entity::note;
fn a() {
    let rows = note::Entity::find()
        .filter(note::Column::Title.eq(\"x\"))
        .all(db)
        .await?;
    note::Entity::update_many().col_expr(a, b).exec(db).await?;
}
";
        let found = check(service);
        let locations: Vec<&str> = found.iter().map(|v| v.location.as_str()).collect();
        assert_eq!(locations, ["notes/src/service.rs:3", "notes/src/service.rs:7"]);
        assert!(found[0].message.contains("add `.scoped(&tenant)`"), "{}", found[0].message);
        assert!(found[0].message.contains("// tenant-scope: <why>"), "{}", found[0].message);
    }

    #[test]
    fn entity_tran_tra_qua_use_va_alias() {
        let service = "use crate::entity::note::{Column, Entity};
use crate::entity::note::Entity as Notes;
fn a() {
    let x = Entity::find().all(db).await?;
    let y = Notes::find_by_id(id).one(db).await?;
}
";
        assert_eq!(check(service).len(), 2);
        // Trong chính file định nghĩa entity.
        let own = format!("{ENTITY}fn b() {{ note::Entity::find().all(db); }}\n");
        let found = check_crate(&[file("notes/src/entity.rs", &own)]);
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn chu_thich_mien_tru_va_ten_mo_ho() {
        let service = "fn a() {
    // tenant-scope: job dọn dẹp chạy cho mọi khách, lọc theo tenant_id từng lô.
    let rows = note::Entity::find().all(db).await?;
}
";
        assert!(check(service).is_empty());
        // `note` có hai entity, chỉ một cài TenantScoped → mơ hồ, bỏ qua.
        let other = "pub mod note {\n    #[sea_orm(table_name = \"x_other\")]\n    pub struct Model {}\n}\n";
        let files = [
            file("notes/src/entity.rs", ENTITY),
            file("notes/src/other.rs", other),
            file("notes/src/service.rs", "fn a() { note::Entity::find().all(db); }\n"),
        ];
        assert!(check_crate(&files).is_empty());
    }

    #[test]
    fn bo_qua_comment_test_va_goi_method() {
        let service = "/// `note::Entity::find()` trong tài liệu.
fn a() { let v = items.iter().find(|x| x).map(Vec::<u8>::new); }
#[cfg(test)]
mod tests {
    fn t() { note::Entity::find().all(db); }
}
";
        assert!(check(service).is_empty());
    }

    #[test]
    fn entity_khai_bang_macro_rieng_cung_bi_kiem() {
        let entity = "macro_rules! tenant_scoped {
    () => {
        impl autoxide_kernel::tenancy::TenantScoped for Entity {
            fn tenant_column() -> Self::Column { Column::TenantId }
        }
    };
}
pub(crate) mod order {
    #[sea_orm(table_name = \"shop_orders\")]
    pub struct Model { pub tenant_id: Uuid }
    tenant_scoped!();
}
pub(crate) mod status {
    #[sea_orm(table_name = \"shop_statuses\")]
    pub struct Model {}
}
";
        let service = "use crate::entity::{order, status};
fn a(t: &TenantContext) {
    order::Entity::delete_by_id(id).exec(db).await?;
    status::Entity::find_by_id(id).one(db).await?;
}
";
        let files = [file("shop/src/entity.rs", entity), file("shop/src/service.rs", service)];
        let violations = check_crate(&files);
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert!(violations[0].message.contains("`order`"), "{}", violations[0].message);
    }

    #[test]
    fn ten_module_cua_file() {
        assert_eq!(file_module("entity/note.rs"), "note");
        assert_eq!(file_module("entity/mod.rs"), "entity");
        assert_eq!(file_module("lib.rs"), "");
    }
}
