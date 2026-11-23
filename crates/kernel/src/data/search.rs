//! Text search in lists: "contains this string", case-insensitive and diacritic-insensitive ("da lat"
//! finds `Đà Lạt`), with the same results on SQLite and Postgres.
//!
//! The shared approach for every module:
//!
//! 1. Store a "search key" column holding text passed through [`fold`] (e.g. `search_key`), updated at the
//!    same time as the source column. This is done in Rust rather than with the database `LOWER()` /
//!    `unaccent()`: SQLite's `LOWER()` only knows ASCII, Postgres's follows the locale; `unaccent` exists
//!    only on Postgres, so the two databases would give two different results.
//! 2. Filter with [`contains`] on that column: `LIKE '%…%' ESCAPE '\'`, so a `%` or `_` typed by the user is
//!    an ordinary character, not a wildcard.
//!
//! [`normalize`] (lowercase only) is the key for name uniqueness and sorting: `Anh` and `Ảnh` are two
//! different words so they must not count as duplicates. If [`fold`] changes, add a migration that
//! recomputes every stored search key column.
//!
//! "Contains" cannot use an index: the database scans the rows left after the other filters. That suffices
//! for tables of a few hundred thousand rows; for larger ones, a search index can replace it
//! ([`SearchIndex`], step 40 P7): the module declares document kinds, and the engine is the default
//! [`DatabaseSearch`] or an external one.

mod backend;
pub(crate) mod database;
mod index;

pub use backend::{
    OrgScope, SearchBackend, SearchDoc, SearchHit, SearchHits, SearchKind, SearchRequest,
    SearchSource,
};
pub use database::DatabaseSearch;
pub use index::{DATABASE_BACKEND, SearchIndex};
pub(crate) use index::{SEARCH_BACKEND, SEARCH_SYNC, SyncRunner, reindex_task};

use sea_orm::ColumnTrait;
use sea_orm::sea_query::{Expr, LikeExpr};
use unicode_normalization::UnicodeNormalization as _;
use unicode_normalization::char::is_combining_mark;

/// Key for uniqueness and text sorting: trims surrounding whitespace and lowercases.
pub fn normalize(text: &str) -> String {
    text.trim().to_lowercase()
}

/// The form compared when searching: like [`normalize`], then strips diacritics: separates letters from
/// marks (Unicode NFD), drops the marks, `đ` → `d`. `Đường đến Đà Lạt` → `duong den da lat`.
pub fn fold(text: &str) -> String {
    normalize(text)
        .nfd()
        .filter(|character| !is_combining_mark(*character))
        .map(|character| if character == 'đ' { 'd' } else { character })
        .collect()
}

/// The form compared in "exact diacritic" matching: like [`normalize`], then merges letters and marks into
/// one form (Unicode NFC), so `Ảnh` typed on one machine or another gives the same string, while "anh" differs.
/// Used as the ranking tier before searching (D-202): in Vietnamese, `dấu / dầu / dâu` mean different things.
pub fn exact_key(text: &str) -> String {
    normalize(text).nfc().collect()
}

/// Condition "column `column` (stored in [`exact_key`] form) contains `text` with exact diacritics".
pub fn contains_exact<C: ColumnTrait>(column: C, text: &str) -> Expr {
    let pattern = format!("%{}%", escape_like(&exact_key(text)));
    column.like(LikeExpr::new(pattern).escape('\\'))
}

/// Condition "column `column` (stored in [`fold`] form) contains `text`"; `text` may have diacritics or
/// not.
pub fn contains<C: ColumnTrait>(column: C, text: &str) -> Expr {
    let pattern = format!("%{}%", escape_like(&fold(text)));
    column.like(LikeExpr::new(pattern).escape('\\'))
}

/// Condition "column `column` starts with exactly the string `prefix`" (compared verbatim, no diacritic
/// folding). Replaces SeaORM's `ColumnTrait::starts_with`, which does not escape `%` / `_`, so a user-chosen
/// `prefix` (slug, reference code) would become a wildcard. Behaves the same on SQLite and Postgres.
pub fn starts_with<C: ColumnTrait>(column: C, prefix: &str) -> Expr {
    let pattern = format!("{}%", escape_like(prefix));
    column.like(LikeExpr::new(pattern).escape('\\'))
}

/// Escapes `\`, `%` and `_` for use in a `LIKE … ESCAPE '\'` pattern. Every `LIKE` pattern built from user
/// data goes through here (or through [`contains`], [`starts_with`]).
pub fn escape_like(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if matches!(character, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thoat_ky_tu_dai_dien() {
        assert_eq!(escape_like(r"50%_a\b"), r"50\%\_a\\b");
        assert_eq!(normalize("  Ảnh ĐẸP "), "ảnh đẹp");
        assert_eq!(fold(" Đường đến Đà Lạt "), "duong den da lat");
        assert_eq!(fold("Ảnh ĐẸP, Crème brûlée"), "anh dep, creme brulee");
        assert_eq!(fold("東京"), "東京", "scripts without accents stay as they are");
        // "Ả" viết sẵn một ký tự hay "A" + dấu hỏi rời: cùng khoá.
        assert_eq!(exact_key(" A\u{0309}nh ĐẸP "), exact_key("ảnh đẹp"));
        assert_ne!(exact_key("ảnh"), exact_key("anh"));
    }
}
