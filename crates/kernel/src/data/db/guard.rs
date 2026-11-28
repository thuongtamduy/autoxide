//! Runtime tenant guard (D-20261128-tenant-runtime-guard): every statement a tenant handle (`tenant.db()`) sends
//! to the database is checked first. A statement that touches a tenant table must filter that table by the
//! handle's tenant (`tenant_id = <the tenant>`), or, for an `INSERT`, write `tenant_id` with the tenant bound.
//! A statement that does not is refused before it reaches the database: the caller gets an internal error,
//! the log names the table, and the metric `kernel_tenant_guard_blocked_total` counts it.
//!
//! The query builder of `db` already adds the filter, so for its statements the guard is a second line. It is
//! the only line for SeaORM statements a module still runs on a tenant handle (the transition bridge) and for
//! hand-written SQL (`handle.sql(…)`), whose text the kernel cannot build itself.
//!
//! What the guard catches: a forgotten filter (the bug class the source lint `tenant-scope` looks for), on both
//! databases, at run time, whatever way the statement was built. What it does not claim: it reads the SQL
//! text, so a filter that is present but defeated (`… OR 1 = 1`) passes. Module code runs in the same process
//! and is trusted; the guard stops mistakes, not a hostile module.

use std::collections::BTreeSet;
use std::sync::{OnceLock, PoisonError, RwLock};

use sea_orm::sea_query::Value;
use sea_orm::{DbBackend, Statement};
use uuid::Uuid;

use crate::error::Error;

// Sổ bảng cấp khách của tiến trình: bảng của mọi record `scope = tenant` mà module khai (`Module::records`, ghi khi
// dựng `Modules`), cộng mọi bảng có cột `tenant_id` trong schema thật trừ bảng control-plane (ghi ở
// `Modules::check_records`, sau migration); chỉ thêm. Đọc ở mỗi câu của handle theo khách — khoá đọc rẻ.
static TENANT_TABLES: OnceLock<RwLock<BTreeSet<String>>> = OnceLock::new();

fn tenant_tables() -> &'static RwLock<BTreeSet<String>> {
    TENANT_TABLES.get_or_init(|| RwLock::new(BTreeSet::new()))
}

/// Records `tables` as tenant tables (called when the modules are built, from the declared records).
pub(crate) fn register_tenant_tables<T: Into<String>>(tables: impl IntoIterator<Item = T>) {
    let mut set = tenant_tables().write().unwrap_or_else(PoisonError::into_inner);
    set.extend(tables.into_iter().map(Into::into));
}

/// Registers every table of the live schema of `database` that has a `tenant_id` column as a tenant table, except
/// the control-plane tables modules declare: the guard then covers the tables of modules not moved to `db` records
/// yet, and the kernel's own tenant tables. Called by `Modules::check_records` once the migrations are applied.
pub(crate) async fn register_live_tenant_tables(
    database: &crate::database::Database,
) -> Result<(), String> {
    use sea_orm::ConnectionTrait as _;
    let connection = database.read();
    let backend = connection.get_database_backend();
    let sql = match backend {
        DbBackend::Sqlite => {
            "SELECT m.name AS name FROM sqlite_master m JOIN pragma_table_info(m.name) p \
             WHERE m.type = 'table' AND p.name = 'tenant_id'"
        }
        DbBackend::Postgres => {
            "SELECT table_name::text AS name FROM information_schema.columns \
             WHERE table_schema = current_schema() AND column_name = 'tenant_id'"
        }
        other => return Err(format!("unsupported database {other:?}")),
    };
    let rows = connection
        .query_all_raw(crate::database::statement(backend, sql, Vec::<Value>::new()))
        .await
        .map_err(|error| error.to_string())?;
    let mut tables = Vec::new();
    for row in &rows {
        let name = row.try_get::<String>("", "name").map_err(|error| error.to_string())?;
        if !super::check::is_control_plane(&name) {
            tables.push(name.to_ascii_lowercase());
        }
    }
    register_tenant_tables(tables);
    Ok(())
}

/// Only for tests: registers tenant tables without building modules.
#[cfg(feature = "testing")]
#[doc(hidden)]
pub fn register_tenant_tables_for_tests(tables: &[&'static str]) {
    register_tenant_tables(tables.iter().copied());
}

/// Checks `statement` before a handle of tenant `tenant` runs it; `Err` (internal) when it touches a tenant
/// table without the tenant filter. No tenant (control plane, all tenants): nothing to check.
pub(crate) fn check(tenant: Option<Uuid>, statement: &Statement) -> Result<(), Error> {
    let Some(tenant) = tenant else {
        return Ok(());
    };
    let registered = tenant_tables().read().unwrap_or_else(PoisonError::into_inner);
    if registered.is_empty() {
        return Ok(());
    }
    let values: &[Value] = statement.values.as_ref().map_or(&[], |values| values.0.as_slice());
    match problem(&statement.sql, values, statement.db_backend, tenant, &registered) {
        None => Ok(()),
        Some(table) => {
            drop(registered);
            metrics::counter!("kernel_tenant_guard_blocked_total").increment(1);
            // Chữ câu có tham số `?`/`$n`, không có giá trị: ghi log được, giúp tìm câu thiếu lọc.
            let sql: String = statement.sql.chars().take(400).collect();
            tracing::error!(
                table = %table,
                sql = %sql,
                "a statement on a tenant handle touches a tenant table without filtering by the tenant; refused"
            );
            Err(Error::internal(format!(
                "statement on a tenant handle reads or writes `{table}` without `tenant_id = <tenant>`; \
                 build it with the `db` query builder or add the tenant filter: {sql}"
            )))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    // Tên (bảng, cột, từ khoá), chữ thường; tên trong ngoặc kép giữ nguyên chữ.
    Name(String),
    Dot,
    Equals,
    // Chỉ số (từ 0) của giá trị được gắn.
    Placeholder(usize),
    Other,
}

// Tách câu thành token, bỏ chuỗi '…' và comment. `?` là tham số trên SQLite, `$n` trên Postgres (cả hai do
// sea-query / `db.sql` sinh). Không phải bộ phân tích SQL: chỉ đủ để thấy tên bảng và `tenant_id = <tham số>`.
fn tokens(sql: &str, backend: DbBackend) -> Vec<Token> {
    let characters: Vec<char> = sql.chars().collect();
    let mut tokens = Vec::new();
    let mut question_marks = 0;
    let mut index = 0;
    while index < characters.len() {
        let character = characters[index];
        let next = characters.get(index + 1).copied();
        match character {
            '\'' => {
                // Chuỗi: `''` bên trong là một dấu nháy.
                index += 1;
                while index < characters.len() {
                    if characters[index] == '\'' {
                        if characters.get(index + 1) == Some(&'\'') {
                            index += 2;
                            continue;
                        }
                        break;
                    }
                    index += 1;
                }
                index += 1;
                tokens.push(Token::Other);
            }
            '-' if next == Some('-') => {
                while index < characters.len() && characters[index] != '\n' {
                    index += 1;
                }
            }
            '/' if next == Some('*') => {
                index += 2;
                while index + 1 < characters.len()
                    && !(characters[index] == '*' && characters[index + 1] == '/')
                {
                    index += 1;
                }
                index += 2;
            }
            '"' => {
                let mut name = String::new();
                index += 1;
                while index < characters.len() {
                    if characters[index] == '"' {
                        if characters.get(index + 1) == Some(&'"') {
                            name.push('"');
                            index += 2;
                            continue;
                        }
                        break;
                    }
                    name.push(characters[index]);
                    index += 1;
                }
                index += 1;
                tokens.push(Token::Name(name));
            }
            ':' if next == Some(':') => {
                // Ép kiểu của Postgres (`$1::uuid`): không đổi giá trị, bỏ cả tên kiểu.
                index += 2;
                while characters.get(index).is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_')
                {
                    index += 1;
                }
            }
            '.' => {
                tokens.push(Token::Dot);
                index += 1;
            }
            '=' => {
                tokens.push(Token::Equals);
                index += 1;
            }
            '?' if backend != DbBackend::Postgres => {
                tokens.push(Token::Placeholder(question_marks));
                question_marks += 1;
                index += 1;
            }
            '$' if backend == DbBackend::Postgres && next.is_some_and(|c| c.is_ascii_digit()) => {
                index += 1;
                let mut number = 0_usize;
                while let Some(digit) = characters.get(index).and_then(|c| c.to_digit(10)) {
                    number = number.saturating_mul(10).saturating_add(digit as usize);
                    index += 1;
                }
                tokens.push(Token::Placeholder(number.saturating_sub(1)));
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let mut name = String::new();
                while let Some(&c) = characters.get(index) {
                    if c.is_ascii_alphanumeric() || c == '_' {
                        name.push(c.to_ascii_lowercase());
                        index += 1;
                    } else {
                        break;
                    }
                }
                tokens.push(Token::Name(name));
            }
            c if c.is_whitespace() => index += 1,
            _ => {
                // Số và dấu khác: một token (`<>`, `>=`… không phải `=` đứng riêng, nên `a >= b` không thành lọc).
                let starts_number = character.is_ascii_digit();
                index += 1;
                if starts_number {
                    while characters.get(index).is_some_and(char::is_ascii_alphanumeric) {
                        index += 1;
                    }
                }
                if !matches!(tokens.last(), Some(Token::Other)) || starts_number {
                    tokens.push(Token::Other);
                }
            }
        }
    }
    tokens
}

// Từ không bao giờ là alias của bảng (từ khoá có thể đứng ngay sau tên bảng).
const NOT_ALIAS: &[&str] = &[
    "all",
    "and",
    "as",
    "asc",
    "by",
    "case",
    "conflict",
    "cross",
    "default",
    "delete",
    "desc",
    "distinct",
    "do",
    "else",
    "end",
    "except",
    "exists",
    "for",
    "from",
    "full",
    "group",
    "having",
    "in",
    "inner",
    "insert",
    "intersect",
    "into",
    "is",
    "join",
    "lateral",
    "left",
    "limit",
    "natural",
    "not",
    "nulls",
    "offset",
    "on",
    "only",
    "or",
    "order",
    "outer",
    "returning",
    "right",
    "select",
    "set",
    "then",
    "union",
    "update",
    "using",
    "values",
    "when",
    "where",
    "window",
    "with",
];

/// One place a tenant table is used as a table (not as the qualifier of a column).
struct Reference {
    table: String,
    /// The name the statement filters it by: its alias, or the table's own name.
    binding: String,
    /// The target of `INSERT INTO`.
    into: bool,
}

// Bảng cấp khách đầu tiên mà câu chạm mà không lọc; `None` khi câu đạt. Đếm theo TỪNG LẦN bảng được dùng làm bảng
// (sau FROM / JOIN / UPDATE / INTO / USING, trong subquery, ở mỗi vế UNION…), không theo số bảng khác nhau: mỗi lần
// phải có một vế `<tên gắn>.tenant_id = <tham số của khách>` riêng (tên gắn = alias, hay chính tên bảng), hay một vế
// không ghi bảng `tenant_id = <tham số>` (mỗi vế như vậy chỉ dùng được cho một lần).
fn problem(
    sql: &str,
    values: &[Value],
    backend: DbBackend,
    tenant: Uuid,
    registered: &BTreeSet<String>,
) -> Option<String> {
    let tokens = tokens(sql, backend);
    let name_at = |index: usize| match tokens.get(index) {
        Some(Token::Name(name)) => Some(name.as_str()),
        _ => None,
    };
    let mut references = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        let Token::Name(name) = token else { continue };
        // `"t"."cột"`: tên bảng làm tiền tố của cột, không phải một lần dùng bảng.
        if !registered.contains(name.as_str()) || tokens.get(index + 1) == Some(&Token::Dot) {
            continue;
        }
        let into = index.checked_sub(1).and_then(name_at) == Some("into");
        let mut next = index + 1;
        if name_at(next) == Some("as") {
            next += 1;
        }
        let binding = match name_at(next) {
            Some(alias) if !NOT_ALIAS.contains(&alias) && !registered.contains(alias) => {
                alias.to_owned()
            }
            _ => name.clone(),
        };
        references.push(Reference { table: name.clone(), binding, into });
    }
    if references.is_empty() {
        return None;
    }
    let is_tenant = |index: usize| matches!(values.get(index), Some(Value::Uuid(Some(value))) if *value == tenant);
    let tenant_bound =
        values.iter().any(|value| matches!(value, Value::Uuid(Some(v)) if *v == tenant));
    // Mỗi vế `tenant_id = <tham số của khách>` (hay đảo hai vế), với tên gắn ở trước dấu chấm nếu có.
    let mut qualified: Vec<String> = Vec::new();
    let mut unqualified = 0_usize;
    let column_at = |index: usize| name_at(index) == Some("tenant_id");
    let bound_at =
        |index: usize| matches!(tokens.get(index), Some(Token::Placeholder(i)) if is_tenant(*i));
    for (index, token) in tokens.iter().enumerate() {
        if *token != Token::Equals || index == 0 {
            continue;
        }
        let column = if column_at(index - 1) && bound_at(index + 1) {
            Some(index - 1)
        } else if bound_at(index - 1) && column_at(index + 1) {
            Some(index + 1)
        } else {
            None
        };
        let Some(column) = column else { continue };
        let qualifier = column
            .checked_sub(2)
            .filter(|_| tokens.get(column - 1) == Some(&Token::Dot))
            .and_then(name_at);
        match qualifier {
            Some(qualifier) => qualified.push(qualifier.to_owned()),
            None => unqualified += 1,
        }
    }
    let names_column =
        tokens.iter().any(|token| matches!(token, Token::Name(name) if name == "tenant_id"));
    // `ON CONFLICT … DO UPDATE`: nhánh cập nhật chạm dòng đang có, nên cũng phải lọc khách.
    let do_update = tokens
        .windows(2)
        .any(|pair| matches!(pair, [Token::Name(a), Token::Name(b)] if a == "do" && b == "update"));
    for reference in &references {
        if reference.into {
            // Bảng đích của `INSERT`: ghi cột `tenant_id` với giá trị của khách.
            if !(names_column && tenant_bound) {
                return Some(reference.table.clone());
            }
            if !do_update {
                continue;
            }
        }
        if let Some(found) = qualified.iter().position(|qualifier| *qualifier == reference.binding)
        {
            qualified.swap_remove(found);
        } else if unqualified > 0 {
            unqualified -= 1;
        } else {
            return Some(reference.table.clone());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const TENANT: Uuid = Uuid::from_u128(7);
    const OTHER: Uuid = Uuid::from_u128(8);

    fn registered() -> BTreeSet<String> {
        ["notes_note", "notes_tag"].into_iter().map(str::to_owned).collect()
    }

    fn uuid(value: Uuid) -> Value {
        Value::Uuid(Some(value))
    }

    fn sqlite(sql: &str, values: &[Value]) -> Option<String> {
        problem(sql, values, DbBackend::Sqlite, TENANT, &registered())
    }

    fn postgres(sql: &str, values: &[Value]) -> Option<String> {
        problem(sql, values, DbBackend::Postgres, TENANT, &registered())
    }

    #[test]
    fn cau_cua_bo_dung_cau_dat_tren_ca_hai_database() {
        let select = r#"SELECT "notes_note"."id" FROM "notes_note" WHERE "notes_note"."id" = ? AND "notes_note"."tenant_id" = ?"#;
        assert_eq!(sqlite(select, &[uuid(OTHER), uuid(TENANT)]), None);
        let select = select.replacen('?', "$1", 1).replacen('?', "$2", 1);
        assert_eq!(postgres(&select, &[uuid(OTHER), uuid(TENANT)]), None);
        let update = r#"UPDATE "notes_note" SET "title" = $1 WHERE "notes_note"."tenant_id" = $2"#;
        assert_eq!(postgres(update, &["x".into(), uuid(TENANT)]), None);
        let insert = r#"INSERT INTO "notes_note" ("id", "tenant_id", "title") VALUES (?, ?, ?)"#;
        assert_eq!(sqlite(insert, &[uuid(OTHER), uuid(TENANT), "x".into()]), None);
        // Bảng không phải cấp khách: không kiểm.
        assert_eq!(sqlite(r#"SELECT 1 FROM "kernel_outbox""#, &[]), None);
    }

    #[test]
    fn thieu_loc_khach_bi_chan() {
        // Chọn cột `tenant_id` nhưng không lọc theo nó.
        let select =
            r#"SELECT "notes_note"."tenant_id" FROM "notes_note" WHERE "notes_note"."id" = ?"#;
        assert_eq!(sqlite(select, &[uuid(TENANT)]).as_deref(), Some("notes_note"));
        // Lọc theo khách khác.
        let other = r#"DELETE FROM "notes_note" WHERE "notes_note"."tenant_id" = $1"#;
        assert_eq!(postgres(other, &[uuid(OTHER)]).as_deref(), Some("notes_note"));
        // Nối hai bảng của khách, chỉ lọc một.
        let join = r#"SELECT 1 FROM "notes_note" JOIN "notes_tag" ON "notes_tag"."note_id" = "notes_note"."id" WHERE "notes_note"."tenant_id" = ?"#;
        assert_eq!(sqlite(join, &[uuid(TENANT)]).as_deref(), Some("notes_tag"));
        // `INSERT` không ghi `tenant_id`, hay ghi khách khác.
        let insert = r#"INSERT INTO "notes_note" ("id", "title") VALUES (?, ?)"#;
        assert_eq!(sqlite(insert, &[uuid(TENANT), "x".into()]).as_deref(), Some("notes_note"));
        let insert = r#"INSERT INTO "notes_note" ("id", "tenant_id") VALUES (?, ?)"#;
        assert_eq!(sqlite(insert, &[uuid(OTHER), uuid(OTHER)]).as_deref(), Some("notes_note"));
        // `>=` không phải lọc; chữ hoa của SQL viết tay vẫn nhận ra tên bảng.
        let bigger = "SELECT 1 FROM NOTES_NOTE WHERE TENANT_ID >= ?";
        assert_eq!(sqlite(bigger, &[uuid(TENANT)]).as_deref(), Some("notes_note"));
    }

    #[test]
    fn sql_viet_tay_comment_va_chuoi_khong_lua_duoc() {
        let hand =
            "SELECT count(*) FROM notes_note n WHERE n.tenant_id = ? AND n.title = 'tenant_id = ?'";
        assert_eq!(sqlite(hand, &[uuid(TENANT), "x".into()]), None);
        // Lọc nằm trong comment: không tính.
        let commented = "SELECT 1 FROM notes_note -- WHERE tenant_id = ?\n";
        assert_eq!(sqlite(commented, &[uuid(TENANT)]).as_deref(), Some("notes_note"));
        let block = "SELECT 1 FROM notes_note /* tenant_id = $1 */";
        assert_eq!(postgres(block, &[uuid(TENANT)]).as_deref(), Some("notes_note"));
        // Lọc nằm trong chuỗi: không tính.
        let quoted = "SELECT 1 FROM notes_note WHERE title = 'tenant_id = ?' AND id = ?";
        assert_eq!(sqlite(quoted, &[uuid(TENANT)]).as_deref(), Some("notes_note"));
        // Ép kiểu của Postgres và tên trong ngoặc kép có dấu nháy kép bên trong.
        let cast = r#"SELECT "a""b" FROM notes_note WHERE $1::uuid = tenant_id"#;
        assert_eq!(postgres(cast, &[uuid(TENANT)]), None);
    }

    // Bốn câu kiểm độc lập dò ra (PR #624): mỗi lần dùng bảng phải có vế lọc riêng, kể cả khi cùng một bảng.
    #[test]
    fn moi_lan_dung_bang_phai_loc_rieng() {
        let t = || uuid(TENANT);
        // Subquery cùng bảng không lọc: đọc vị trí lớn nhất của MỌI khách.
        let sub = "UPDATE notes_note SET position = (SELECT max(position) FROM notes_note) WHERE tenant_id = ?";
        assert_eq!(sqlite(sub, &[t()]).as_deref(), Some("notes_note"));
        let sub_ok = "UPDATE notes_note SET position = (SELECT max(position) FROM notes_note WHERE tenant_id = ?) WHERE tenant_id = ?";
        assert_eq!(sqlite(sub_ok, &[t(), t()]), None);
        // Nối bảng với chính nó, chỉ lọc một phía.
        let self_join = "SELECT a.id FROM notes_note a JOIN notes_note b ON b.id = a.parent_id WHERE a.tenant_id = $1";
        assert_eq!(postgres(self_join, &[t()]).as_deref(), Some("notes_note"));
        let self_join_ok = "SELECT a.id FROM notes_note AS a JOIN notes_note AS b ON b.id = a.parent_id WHERE a.tenant_id = $1 AND b.tenant_id = $2";
        assert_eq!(postgres(self_join_ok, &[t(), t()]), None);
        // Vế sau của UNION không lọc.
        let union = "SELECT id FROM notes_note WHERE tenant_id = ? UNION SELECT id FROM notes_note";
        assert_eq!(sqlite(union, &[t()]).as_deref(), Some("notes_note"));
        // Nối hai bảng nhưng lọc hai lần trên cùng một alias.
        let twice = "SELECT n.id FROM notes_note n JOIN notes_tag g ON g.note_id = n.id WHERE n.tenant_id = ? AND n.tenant_id = ?";
        assert_eq!(sqlite(twice, &[t(), t()]).as_deref(), Some("notes_tag"));
        let both = "SELECT n.id FROM notes_note n JOIN notes_tag g ON g.note_id = n.id WHERE n.tenant_id = ? AND g.tenant_id = ?";
        assert_eq!(sqlite(both, &[t(), t()]), None);
    }

    #[test]
    fn upsert_do_update_phai_loc_khach() {
        let t = || uuid(TENANT);
        let bare = r#"INSERT INTO "notes_note" ("id", "tenant_id") VALUES (?, ?) ON CONFLICT ("id") DO UPDATE SET "title" = "excluded"."title""#;
        assert_eq!(sqlite(bare, &[uuid(OTHER), t()]).as_deref(), Some("notes_note"));
        let guarded = r#"INSERT INTO "notes_note" ("id", "tenant_id") VALUES (?, ?) ON CONFLICT ("id") DO UPDATE SET "title" = "excluded"."title" WHERE "notes_note"."tenant_id" = ?"#;
        assert_eq!(sqlite(guarded, &[uuid(OTHER), t(), t()]), None);
        let nothing = r#"INSERT INTO "notes_note" ("id", "tenant_id") VALUES (?, ?) ON CONFLICT ("id") DO NOTHING"#;
        assert_eq!(sqlite(nothing, &[uuid(OTHER), t()]), None);
    }

    // Bộ câu dò của kiểm độc lập lần 2 (PR #624): 10 câu phải bị chặn, 1 câu (EXISTS sang bảng khác) cũng bị chặn; mỗi
    // câu thử với cả cách viết tham số của SQLite (`?`) và Postgres (`$n`).
    #[test]
    fn bo_cau_do_kiem_doc_lap() {
        let probes: [(&str, usize); 11] = [
            (
                "UPDATE notes_note SET position = (SELECT max(position) FROM notes_note) WHERE tenant_id = ?",
                1,
            ),
            (
                "SELECT a.id FROM notes_note a JOIN notes_note b ON b.id = a.parent_id WHERE a.tenant_id = ?",
                1,
            ),
            ("SELECT id FROM notes_note WHERE tenant_id = ? UNION SELECT id FROM notes_note", 1),
            (
                "SELECT n.id FROM notes_note n JOIN notes_tag g ON g.note_id = n.id WHERE n.tenant_id = ? AND n.tenant_id = ?",
                2,
            ),
            (
                "WITH x AS (SELECT id FROM notes_note) SELECT id FROM notes_note WHERE tenant_id = ? AND id IN (SELECT id FROM x)",
                1,
            ),
            (
                "SELECT id FROM notes_note WHERE tenant_id = ? AND parent_id IN (SELECT id FROM notes_note)",
                1,
            ),
            (
                "SELECT id FROM notes_note n WHERE n.tenant_id = ? AND EXISTS (SELECT 1 FROM notes_note m WHERE m.id = n.parent_id)",
                1,
            ),
            (
                "INSERT INTO notes_note (id, tenant_id, title) SELECT id, ?, title FROM notes_note",
                1,
            ),
            (
                "DELETE FROM notes_note USING notes_tag WHERE notes_note.tenant_id = ? AND notes_tag.note_id = notes_note.id",
                1,
            ),
            (
                "INSERT INTO notes_note (id, tenant_id) VALUES (?, ?) ON CONFLICT (id) DO UPDATE SET title = excluded.title",
                2,
            ),
            (
                "SELECT id FROM notes_note n WHERE n.tenant_id = ? AND EXISTS (SELECT 1 FROM notes_tag g WHERE g.note_id = n.id)",
                1,
            ),
        ];
        for (sql, count) in probes {
            let values = vec![uuid(TENANT); count];
            assert!(sqlite(sql, &values).is_some(), "SQLite let through: {sql}");
            let mut numbered = String::new();
            let mut n = 0;
            for c in sql.chars() {
                if c == '?' {
                    n += 1;
                    numbered.push_str(&format!("${n}"));
                } else {
                    numbered.push(c);
                }
            }
            assert!(postgres(&numbered, &values).is_some(), "Postgres let through: {numbered}");
        }
    }
}
